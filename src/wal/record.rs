//! Formato binário e serialização de registros do Write-Ahead Log (WAL).
//!
//! Layout do registro em disco (Little-Endian):
//! ```text
//! +---------------+------------+--------------+---------------+-----------------+
//! | CRC32 (4 B)   | Flags (1B) | Key (8 B)    | ValLen (4 B)  | Value (N Bytes) |
//! +---------------+------------+--------------+---------------+-----------------+
//! ```

use std::io::{self, Read, Write};

use crc32fast::Hasher;

use crate::error::{EngineError, Key, Result};

/// Tamanho do cabeçalho binário em bytes:
/// CRC32 (4) + Flags (1) + Key (8) + ValLen (4) = 17 bytes.
pub const HEADER_SIZE: usize = 17;

/// Flag para operação PUT (inserção / sobrescrita de registro ativo).
pub const FLAG_PUT: u8 = 0x01;

/// Flag para operação DELETE (marcador de exclusão / tombstone).
pub const FLAG_DELETE: u8 = 0x02;

/// Limite máximo defensivo para o tamanho do valor de um registro (64 MB).
/// Previne alocações de memória descontroladas causadas por bytes de comprimento corrompidos.
pub const MAX_VALUE_LEN: u32 = 64 * 1024 * 1024;

/// Registro de dados do Write-Ahead Log em memória.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// Chave identificadora inteira sem sinal de 64 bits.
    pub key: Key,
    /// Vetor de bytes contendo o payload do valor.
    pub value: Vec<u8>,
    /// Indica se o registro é um marcador de remoção (tombstone).
    pub is_tombstone: bool,
}

impl LogRecord {
    /// Cria um novo registro ativo de inserção/atualização (`PUT`).
    pub fn put(key: Key, value: Vec<u8>) -> Self {
        Self {
            key,
            value,
            is_tombstone: false,
        }
    }

    /// Cria um novo registro marcador de exclusão (`DELETE`).
    pub fn delete(key: Key) -> Self {
        Self {
            key,
            value: Vec::new(),
            is_tombstone: true,
        }
    }

    /// Retorna verdadeiro se o registro for um tombstone.
    pub fn is_tombstone(&self) -> bool {
        self.is_tombstone
    }

    /// Retorna a flag binária correspondente ao tipo do registro.
    pub fn flag(&self) -> u8 {
        if self.is_tombstone {
            FLAG_DELETE
        } else {
            FLAG_PUT
        }
    }

    /// Retorna o tamanho total serializado deste registro em bytes (cabeçalho + valor).
    pub fn encoded_size(&self) -> usize {
        HEADER_SIZE + if self.is_tombstone { 0 } else { self.value.len() }
    }

    /// Calcula o checksum CRC32 dos campos do registro (Flags + Key + ValLen + Value).
    pub fn compute_crc(&self) -> u32 {
        let flag = self.flag();
        let val_len = if self.is_tombstone { 0 } else { self.value.len() as u32 };
        let value_slice = if self.is_tombstone { &[] } else { self.value.as_slice() };
        calculate_crc(flag, self.key, val_len, value_slice)
    }

    /// Serializa o registro binário diretamente em um writer (`io::Write`).
    /// Retorna o número total de bytes gravados (`HEADER_SIZE + value_len`).
    pub fn encode<W: Write>(&self, writer: &mut W) -> Result<usize> {
        let flag = self.flag();
        let val_len = if self.is_tombstone {
            0u32
        } else {
            self.value.len() as u32
        };
        let value_slice = if self.is_tombstone {
            &[]
        } else {
            self.value.as_slice()
        };

        let crc = calculate_crc(flag, self.key, val_len, value_slice);

        let mut header = [0u8; HEADER_SIZE];
        header[0..4].copy_from_slice(&crc.to_le_bytes());
        header[4] = flag;
        header[5..13].copy_from_slice(&self.key.to_le_bytes());
        header[13..17].copy_from_slice(&val_len.to_le_bytes());

        writer.write_all(&header)?;
        if val_len > 0 {
            writer.write_all(value_slice)?;
        }

        Ok(HEADER_SIZE + val_len as usize)
    }

    /// Serializa o registro binário para um novo vetor de bytes (`Vec<u8>`).
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_size());
        self.encode(&mut buf)
            .expect("escrita em Vec<u8> em memória não deve falhar");
        buf
    }

    /// Deserializa um registro a partir de um reader sequencial (`io::Read`).
    ///
    /// # Retornos
    /// - `Ok(Some(record))`: Registro lido e validado com sucesso.
    /// - `Ok(None)`: Fim de arquivo limpo atingido (0 bytes antes de iniciar um registro).
    /// - `Err(EngineError::UnexpectedEof)`: Gravação truncada no meio do cabeçalho ou payload.
    /// - `Err(EngineError::CrcMismatch)`: Checksum CRC32 divergente no offset especificado.
    /// - `Err(EngineError::InvalidRecord)`: Dados estruturais inválidos (flags desconhecidas, comprimentos exorbitantes).
    /// - `Err(EngineError::Io)`: Erro genérico de I/O de disco.
    pub fn decode<R: Read>(reader: &mut R, current_offset: u64) -> Result<Option<Self>> {
        let mut header = [0u8; HEADER_SIZE];

        // Tenta ler o primeiro byte do cabeçalho para diferenciar EOF limpo de escrita parcial
        match reader.read_exact(&mut header[0..1]) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(EngineError::Io(e)),
        }

        // Lê os 16 bytes restantes do cabeçalho
        if let Err(e) = reader.read_exact(&mut header[1..HEADER_SIZE]) {
            return if e.kind() == io::ErrorKind::UnexpectedEof {
                Err(EngineError::UnexpectedEof)
            } else {
                Err(EngineError::Io(e))
            };
        }

        let expected_crc = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let flag = header[4];
        let key = u64::from_le_bytes(header[5..13].try_into().unwrap());
        let val_len = u32::from_le_bytes(header[13..17].try_into().unwrap());

        if flag != FLAG_PUT && flag != FLAG_DELETE {
            return Err(EngineError::InvalidRecord(format!(
                "Flag de registro desconhecida ({flag:#04x}) no offset {current_offset}"
            )));
        }

        let is_tombstone = flag == FLAG_DELETE;

        if is_tombstone && val_len != 0 {
            return Err(EngineError::InvalidRecord(format!(
                "Registro de exclusão (tombstone) com val_len={val_len} diferente de zero no offset {current_offset}"
            )));
        }

        if val_len > MAX_VALUE_LEN {
            return Err(EngineError::InvalidRecord(format!(
                "Tamanho do valor ({val_len} bytes) excede o limite máximo permitido ({MAX_VALUE_LEN} bytes) no offset {current_offset}"
            )));
        }

        let mut value = vec![0u8; val_len as usize];
        if val_len > 0 {
            if let Err(e) = reader.read_exact(&mut value) {
                return if e.kind() == io::ErrorKind::UnexpectedEof {
                    Err(EngineError::UnexpectedEof)
                } else {
                    Err(EngineError::Io(e))
                };
            }
        }

        let calculated_crc = calculate_crc(flag, key, val_len, &value);
        if calculated_crc != expected_crc {
            return Err(EngineError::CrcMismatch {
                expected: expected_crc,
                calculated: calculated_crc,
                offset: current_offset,
            });
        }

        Ok(Some(Self {
            key,
            value,
            is_tombstone,
        }))
    }
}

/// Função utilitária para calcular o checksum CRC32 sobre os dados do registro.
pub fn calculate_crc(flag: u8, key: Key, val_len: u32, value: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(&[flag]);
    hasher.update(&key.to_le_bytes());
    hasher.update(&val_len.to_le_bytes());
    hasher.update(value);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_encode_decode_put_roundtrip() {
        let record = LogRecord::put(12345, b"hello world".to_vec());
        let encoded = record.encode_to_vec();

        assert_eq!(encoded.len(), record.encoded_size());
        assert_eq!(encoded.len(), HEADER_SIZE + 11);

        let mut cursor = Cursor::new(encoded);
        let decoded = LogRecord::decode(&mut cursor, 0).unwrap().unwrap();

        assert_eq!(decoded.key, 12345);
        assert_eq!(decoded.value, b"hello world");
        assert!(!decoded.is_tombstone());
    }

    #[test]
    fn test_encode_decode_delete_roundtrip() {
        let record = LogRecord::delete(999);
        let encoded = record.encode_to_vec();

        assert_eq!(encoded.len(), HEADER_SIZE);

        let mut cursor = Cursor::new(encoded);
        let decoded = LogRecord::decode(&mut cursor, 100).unwrap().unwrap();

        assert_eq!(decoded.key, 999);
        assert!(decoded.value.is_empty());
        assert!(decoded.is_tombstone());
    }

    #[test]
    fn test_decode_clean_eof() {
        let mut cursor = Cursor::new(Vec::new());
        let result = LogRecord::decode(&mut cursor, 0).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_decode_unexpected_eof_in_header() {
        // Apenas 10 bytes fornecidos (menos que os 17 do cabeçalho)
        let data = vec![0u8; 10];
        let mut cursor = Cursor::new(data);
        let result = LogRecord::decode(&mut cursor, 0);

        match result {
            Err(EngineError::UnexpectedEof) => (),
            _ => panic!("Esperado UnexpectedEof para cabeçalho incompleto"),
        }
    }

    #[test]
    fn test_decode_unexpected_eof_in_payload() {
        let record = LogRecord::put(42, b"dados longos para simular falha no payload".to_vec());
        let mut encoded = record.encode_to_vec();
        // Trunca os últimos 5 bytes do valor
        encoded.truncate(encoded.len() - 5);

        let mut cursor = Cursor::new(encoded);
        let result = LogRecord::decode(&mut cursor, 50);

        match result {
            Err(EngineError::UnexpectedEof) => (),
            _ => panic!("Esperado UnexpectedEof para payload incompleto"),
        }
    }

    #[test]
    fn test_decode_crc_mismatch() {
        let record = LogRecord::put(100, b"teste de corrupcao".to_vec());
        let mut encoded = record.encode_to_vec();

        // Corrompe um byte do valor
        let last_idx = encoded.len() - 1;
        encoded[last_idx] ^= 0xFF;

        let mut cursor = Cursor::new(encoded);
        let result = LogRecord::decode(&mut cursor, 2048);

        match result {
            Err(EngineError::CrcMismatch {
                expected,
                calculated,
                offset,
            }) => {
                assert_eq!(offset, 2048);
                assert_ne!(expected, calculated);
            }
            _ => panic!("Esperado CrcMismatch"),
        }
    }

    #[test]
    fn test_decode_invalid_flag() {
        let record = LogRecord::put(1, b"val".to_vec());
        let mut encoded = record.encode_to_vec();
        // Modifica a flag (byte 4) para 0x03 (inválida)
        encoded[4] = 0x03;

        let mut cursor = Cursor::new(encoded);
        let result = LogRecord::decode(&mut cursor, 0);

        match result {
            Err(EngineError::InvalidRecord(msg)) => {
                assert!(msg.contains("Flag de registro desconhecida"));
            }
            _ => panic!("Esperado InvalidRecord para flag desconhecida"),
        }
    }

    #[test]
    fn test_decode_delete_with_non_zero_val_len() {
        let record = LogRecord::delete(1);
        let mut encoded = record.encode_to_vec();
        // Modifica val_len (bytes 13..17) para 5
        encoded[13..17].copy_from_slice(&5u32.to_le_bytes());

        let mut cursor = Cursor::new(encoded);
        let result = LogRecord::decode(&mut cursor, 0);

        match result {
            Err(EngineError::InvalidRecord(msg)) => {
                assert!(msg.contains("tombstone"));
            }
            _ => panic!("Esperado InvalidRecord para tombstone com val_len > 0"),
        }
    }
}
