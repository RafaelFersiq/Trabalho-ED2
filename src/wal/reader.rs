//! Leitor sequencial, leitura pontual por offset e recuperação de crash do WAL.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::{EngineError, RecordLocation, Result};
use crate::wal::record::LogRecord;
use crate::wal::writer::WAL_FILE_NAME;

/// Representa uma entrada lida do Write-Ahead Log com seu registro e metadados de localização.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalEntry {
    /// O registro decodificado.
    pub record: LogRecord,
    /// Localização física do registro no arquivo (offset inicial e tamanho do valor).
    pub location: RecordLocation,
    /// Tamanho total codificado em bytes (cabeçalho + valor).
    pub total_size: usize,
}

/// Relatório consolidado da rotina de recuperação e reparo pós-crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Deslocamento em bytes do último registro íntegro confirmado no arquivo.
    pub valid_offset: u64,
    /// Quantidade total de registros válidos recuperados com sucesso.
    pub valid_records_count: usize,
    /// Indica se houve truncamento físico no arquivo devido a gravação parcial no fim.
    pub truncated: bool,
    /// Quantidade de bytes corrompidos/incompletos descartados no truncamento.
    pub truncated_bytes: u64,
}

/// Leitor pontual por offset e utilitário de inspeção física do WAL.
///
/// Utilizado para atender consultas `GET` em $O(1)$ posicionando o cursor
/// via `seek` no offset indicado pelo índice em memória, validando o CRC32 antes de retornar.
pub struct WalReader {
    file: File,
    path: PathBuf,
}

impl WalReader {
    /// Abre o arquivo WAL para leitura no caminho especificado.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).open(&path)?;
        Ok(Self { file, path })
    }

    /// Abre o arquivo WAL padrão (`data.wal`) dentro de um diretório de dados.
    pub fn open_in_dir<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let path = dir.as_ref().join(WAL_FILE_NAME);
        Self::open(path)
    }

    /// Lê e decodifica um registro localizado no offset especificado.
    ///
    /// Valida o cabeçalho e calcula o checksum CRC32 dos bytes em disco.
    /// Retorna erro caso o registro esteja truncado ou ocorra divergência de CRC.
    pub fn read_record_at(&mut self, offset: u64) -> Result<LogRecord> {
        self.file.seek(SeekFrom::Start(offset))?;
        match LogRecord::decode(&mut self.file, offset)? {
            Some(record) => Ok(record),
            None => Err(EngineError::UnexpectedEof),
        }
    }

    /// Lê diretamente o valor associado a uma chave com base em seu `RecordLocation`.
    ///
    /// Executa validação integral de integridade via CRC32.
    pub fn read_value_at(&mut self, location: RecordLocation) -> Result<Vec<u8>> {
        let record = self.read_record_at(location.offset)?;
        if record.is_tombstone {
            return Err(EngineError::Corruption(format!(
                "Tentativa de ler valor de um registro de exclusão (tombstone) no offset {}",
                location.offset
            )));
        }
        Ok(record.value)
    }

    /// Retorna a referência para o caminho do arquivo.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Executa a recuperação pós-crash do arquivo WAL sob `path`.
    ///
    /// Processa cada registro sequencialmente via closure `on_entry` em streaming (evitando carregar
    /// todo o dataset na memória RAM). Se detectar uma gravação parcial ou corrompida no final do
    /// arquivo provocada por encerramento forçado (`SIGKILL`, queda de energia), trunca o arquivo
    /// fisicamente no último byte válido (`valid_offset`).
    ///
    /// Erros de integridade no meio do arquivo NÃO são truncados e retornam erro de corrupção.
    pub fn recover_and_repair<P, F>(path: P, mut on_entry: F) -> Result<RecoveryReport>
    where
        P: AsRef<Path>,
        F: FnMut(WalEntry) -> Result<()>,
    {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(RecoveryReport {
                valid_offset: 0,
                valid_records_count: 0,
                truncated: false,
                truncated_bytes: 0,
            });
        }

        let file = OpenOptions::new().read(true).open(&path)?;
        let initial_file_len = file.metadata()?.len();

        let mut reader = BufReader::new(file);
        let mut valid_offset = 0u64;
        let mut valid_records_count = 0usize;

        loop {
            let record_start_offset = valid_offset;
            match LogRecord::decode(&mut reader, record_start_offset) {
                Ok(Some(record)) => {
                    let total_size = record.encoded_size();
                    let val_len = if record.is_tombstone {
                        0
                    } else {
                        record.value.len() as u32
                    };
                    let location = RecordLocation::new(record_start_offset, val_len);
                    let entry = WalEntry {
                        record,
                        location,
                        total_size,
                    };

                    valid_offset += total_size as u64;
                    valid_records_count += 1;

                    on_entry(entry)?;
                }
                Ok(None) => {
                    // EOF limpo atingido sem resíduos
                    break;
                }
                Err(err) => {
                    // Avalia se o erro ocorreu no final do arquivo (típico de crash durante gravação)
                    let is_at_end_of_file = match &err {
                        EngineError::UnexpectedEof => true,
                        EngineError::CrcMismatch { .. } => {
                            // Se após a leitura do registro com falha de CRC o arquivo atingiu EOF,
                            // o erro ocorreu no último registro do log (queda durante a escrita).
                            // Se houver mais bytes após o registro, a corrupção está no meio do arquivo.
                            let mut probe = [0u8; 1];
                            match std::io::Read::read(&mut reader, &mut probe) {
                                Ok(0) => true,
                                _ => false,
                            }
                        }
                        EngineError::InvalidRecord(_) => {
                            let mut probe = [0u8; 1];
                            match std::io::Read::read(&mut reader, &mut probe) {
                                Ok(0) => true,
                                _ => false,
                            }
                        }
                        _ => false,
                    };

                    if is_at_end_of_file {
                        let truncated_bytes = initial_file_len.saturating_sub(valid_offset);
                        // Trunca o arquivo no último offset íntegro
                        let file_to_truncate = OpenOptions::new()
                            .write(true)
                            .open(&path)?;
                        file_to_truncate.set_len(valid_offset)?;
                        file_to_truncate.sync_all()?;

                        return Ok(RecoveryReport {
                            valid_offset,
                            valid_records_count,
                            truncated: true,
                            truncated_bytes,
                        });
                    }

                    // Se a corrupção não foi no fim do arquivo, é um erro de integridade crítico
                    return Err(err);
                }
            }
        }

        Ok(RecoveryReport {
            valid_offset,
            valid_records_count,
            truncated: false,
            truncated_bytes: 0,
        })
    }

    /// Executa recuperação pós-crash do arquivo WAL padrão (`data.wal`) no diretório indicado.
    pub fn recover_in_dir<P, F>(dir: P, on_entry: F) -> Result<RecoveryReport>
    where
        P: AsRef<Path>,
        F: FnMut(WalEntry) -> Result<()>,
    {
        let path = dir.as_ref().join(WAL_FILE_NAME);
        Self::recover_and_repair(path, on_entry)
    }

    /// Valida integralmente os registros de um arquivo WAL do início ao fim sem realizar alterações.
    ///
    /// Utilizado pelo comando `verify` para auditar a integridade física de todos os dados em disco.
    /// Retorna `(registros_válidos, total_bytes_auditados)`.
    pub fn verify_file<P: AsRef<Path>>(path: P) -> Result<(usize, u64)> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(EngineError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Arquivo WAL não encontrado em {}", path.display()),
            )));
        }

        let file = OpenOptions::new().read(true).open(path)?;
        let mut reader = BufReader::new(file);
        let mut offset = 0u64;
        let mut count = 0usize;

        while let Some(record) = LogRecord::decode(&mut reader, offset)? {
            offset += record.encoded_size() as u64;
            count += 1;
        }

        Ok((count, offset))
    }

    /// Valida a integridade do arquivo WAL padrão no diretório especificado.
    pub fn verify_in_dir<P: AsRef<Path>>(dir: P) -> Result<(usize, u64)> {
        let path = dir.as_ref().join(WAL_FILE_NAME);
        Self::verify_file(path)
    }
}

/// Iterador sequencial sobre registros do Write-Ahead Log.
///
/// Lê entradas individualmente com `BufReader`, permitindo varredura de streaming
/// com baixo consumo de memória RAM.
pub struct WalIterator {
    reader: BufReader<File>,
    current_offset: u64,
}

impl WalIterator {
    /// Cria um novo `WalIterator` abrindo o arquivo no caminho especificado.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = OpenOptions::new().read(true).open(path)?;
        Ok(Self {
            reader: BufReader::new(file),
            current_offset: 0,
        })
    }

    /// Lê a próxima entrada válida do log.
    ///
    /// Retorna `Ok(Some(WalEntry))` se houver registro, `Ok(None)` no fim limpo do arquivo,
    /// ou erro se encontrar corrupção ou fim inesperado.
    pub fn next_entry(&mut self) -> Result<Option<WalEntry>> {
        let start_offset = self.current_offset;
        match LogRecord::decode(&mut self.reader, start_offset)? {
            Some(record) => {
                let total_size = record.encoded_size();
                self.current_offset += total_size as u64;

                let val_len = if record.is_tombstone {
                    0
                } else {
                    record.value.len() as u32
                };

                let location = RecordLocation::new(start_offset, val_len);
                Ok(Some(WalEntry {
                    record,
                    location,
                    total_size,
                }))
            }
            None => Ok(None),
        }
    }

    /// Retorna o offset atual do iterador no arquivo.
    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::record::HEADER_SIZE;
    use crate::wal::writer::WalWriter;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_point_read_at_and_read_value_at() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        let rec1 = LogRecord::put(100, b"valor_100".to_vec());
        let rec2 = LogRecord::put(200, b"valor_200_longo".to_vec());
        let rec3 = LogRecord::delete(100);

        let loc1 = writer.write_and_sync(&rec1).unwrap();
        let loc2 = writer.write_and_sync(&rec2).unwrap();
        let loc3 = writer.write_and_sync(&rec3).unwrap();

        let mut reader = WalReader::open_in_dir(dir.path()).unwrap();

        // 1. Lê registros completos por offset
        let r1 = reader.read_record_at(loc1.offset).unwrap();
        assert_eq!(r1.key, 100);
        assert_eq!(r1.value, b"valor_100");
        assert!(!r1.is_tombstone());

        let r2 = reader.read_record_at(loc2.offset).unwrap();
        assert_eq!(r2.key, 200);
        assert_eq!(r2.value, b"valor_200_longo");

        let r3 = reader.read_record_at(loc3.offset).unwrap();
        assert_eq!(r3.key, 100);
        assert!(r3.is_tombstone());

        // 2. Lê valores diretamente
        let val1 = reader.read_value_at(loc1).unwrap();
        assert_eq!(val1, b"valor_100");

        let val2 = reader.read_value_at(loc2).unwrap();
        assert_eq!(val2, b"valor_200_longo");

        // Tentativa de ler valor de tombstone deve acusar erro
        let val3_err = reader.read_value_at(loc3);
        assert!(val3_err.is_err());
    }

    #[test]
    fn test_point_read_detects_crc_mismatch() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();
        let rec = LogRecord::put(1, b"dados_originais".to_vec());
        let loc = writer.write_and_sync(&rec).unwrap();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // Corrompe um byte do valor no arquivo
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&wal_path)
            .unwrap();
        file.seek(SeekFrom::Start(loc.offset + HEADER_SIZE as u64 + 2))
            .unwrap();
        file.write_all(&[0xFF]).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let mut reader = WalReader::open(&wal_path).unwrap();
        match reader.read_record_at(loc.offset) {
            Err(EngineError::CrcMismatch { offset, .. }) => {
                assert_eq!(offset, loc.offset);
            }
            res => panic!("Esperado CrcMismatch, obtido {:?}", res),
        }
    }

    #[test]
    fn test_wal_iterator_clean() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        let rec1 = LogRecord::put(10, b"dez".to_vec());
        let rec2 = LogRecord::put(20, b"vinte".to_vec());
        let rec3 = LogRecord::delete(10);

        writer.write_and_sync(&rec1).unwrap();
        writer.write_and_sync(&rec2).unwrap();
        writer.write_and_sync(&rec3).unwrap();
        let expected_total_size = writer.current_offset();

        let mut iter = WalIterator::open(writer.path()).unwrap();

        let e1 = iter.next_entry().unwrap().unwrap();
        assert_eq!(e1.record.key, 10);
        assert_eq!(e1.location.offset, 0);

        let e2 = iter.next_entry().unwrap().unwrap();
        assert_eq!(e2.record.key, 20);
        assert_eq!(e2.location.offset, e1.total_size as u64);

        let e3 = iter.next_entry().unwrap().unwrap();
        assert_eq!(e3.record.key, 10);
        assert!(e3.record.is_tombstone());

        assert_eq!(iter.current_offset(), expected_total_size);
        assert!(iter.next_entry().unwrap().is_none());
    }

    #[test]
    fn test_recover_clean_file() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"v1".to_vec()))
            .unwrap();
        writer
            .write_and_sync(&LogRecord::put(2, b"v2".to_vec()))
            .unwrap();
        let file_len = writer.current_offset();
        drop(writer);

        let mut collected_keys = Vec::new();
        let report = WalReader::recover_in_dir(dir.path(), |entry| {
            collected_keys.push(entry.record.key);
            Ok(())
        })
        .unwrap();

        assert_eq!(report.valid_records_count, 2);
        assert_eq!(report.valid_offset, file_len);
        assert!(!report.truncated);
        assert_eq!(report.truncated_bytes, 0);
        assert_eq!(collected_keys, vec![1, 2]);
    }

    #[test]
    fn test_recover_partial_header_crash() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"val1".to_vec()))
            .unwrap();
        let valid_len = writer.current_offset();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // Simula queda de energia no meio da gravação de um cabeçalho (escreve 7 bytes lixo)
        {
            let mut file = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&wal_path)
                .unwrap();
            file.write_all(&[0xAA; 7]).unwrap();
            file.sync_all().unwrap();
        }

        assert_eq!(
            std::fs::metadata(&wal_path).unwrap().len(),
            valid_len + 7
        );

        // Executa recuperação
        let mut count = 0;
        let report = WalReader::recover_and_repair(&wal_path, |_entry| {
            count += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(report.valid_records_count, 1);
        assert_eq!(report.valid_offset, valid_len);
        assert!(report.truncated);
        assert_eq!(report.truncated_bytes, 7);

        // O arquivo físico foi restaurado para o tamanho íntegro
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), valid_len);

        // Garante que o WalWriter pode reabrir e continuar gravando perfeitamente
        let mut new_writer = WalWriter::open(&wal_path).unwrap();
        assert_eq!(new_writer.current_offset(), valid_len);
        let loc = new_writer
            .write_and_sync(&LogRecord::put(2, b"val2".to_vec()))
            .unwrap();
        assert_eq!(loc.offset, valid_len);
    }

    #[test]
    fn test_recover_partial_payload_crash() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"bom".to_vec()))
            .unwrap();
        let valid_len = writer.current_offset();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // Simula escrita de registro cujo cabeçalho diz ter 20 bytes de valor, mas só grava 5 bytes
        let incomplete_rec = LogRecord::put(2, vec![b'X'; 20]);
        let mut encoded = incomplete_rec.encode_to_vec();
        encoded.truncate(HEADER_SIZE + 5);

        {
            let mut file = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&wal_path)
                .unwrap();
            file.write_all(&encoded).unwrap();
            file.sync_all().unwrap();
        }

        let report = WalReader::recover_and_repair(&wal_path, |_| Ok(())).unwrap();
        assert_eq!(report.valid_records_count, 1);
        assert_eq!(report.valid_offset, valid_len);
        assert!(report.truncated);
        assert_eq!(report.truncated_bytes, (HEADER_SIZE + 5) as u64);
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), valid_len);
    }

    #[test]
    fn test_recover_crc_corrupted_last_record() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"primeiro".to_vec()))
            .unwrap();
        let valid_len = writer.current_offset();

        // Grava segundo registro
        let loc2 = writer
            .write_and_sync(&LogRecord::put(2, b"segundo".to_vec()))
            .unwrap();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // Corrompe o payload do segundo registro (que é o último)
        {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&wal_path)
                .unwrap();
            file.seek(SeekFrom::Start(loc2.offset + HEADER_SIZE as u64))
                .unwrap();
            file.write_all(&[0x00]).unwrap();
            file.sync_all().unwrap();
        }

        // A recuperação detecta que a falha é no último registro e o descarta
        let report = WalReader::recover_and_repair(&wal_path, |_| Ok(())).unwrap();
        assert_eq!(report.valid_records_count, 1);
        assert_eq!(report.valid_offset, valid_len);
        assert!(report.truncated);
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), valid_len);
    }

    #[test]
    fn test_corruption_in_middle_returns_error() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"um".to_vec()))
            .unwrap();
        let loc2 = writer
            .write_and_sync(&LogRecord::put(2, b"dois".to_vec()))
            .unwrap();
        writer
            .write_and_sync(&LogRecord::put(3, b"tres".to_vec()))
            .unwrap();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // Corrompe o registro 2 (que está no MEIO do arquivo)
        {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&wal_path)
                .unwrap();
            file.seek(SeekFrom::Start(loc2.offset + HEADER_SIZE as u64))
                .unwrap();
            file.write_all(&[0xFF]).unwrap();
            file.sync_all().unwrap();
        }

        // Tentativa de recuperação deve falhar com erro de integridade e NÃO mascarar com truncamento
        let res = WalReader::recover_and_repair(&wal_path, |_| Ok(()));
        match res {
            Err(EngineError::CrcMismatch { offset, .. }) => {
                assert_eq!(offset, loc2.offset);
            }
            other => panic!("Esperado erro CrcMismatch para corrupção no meio, obtido {:?}", other),
        }
    }

    #[test]
    fn test_verify_file_clean_and_corrupt() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        writer
            .write_and_sync(&LogRecord::put(1, b"alpha".to_vec()))
            .unwrap();
        writer
            .write_and_sync(&LogRecord::put(2, b"beta".to_vec()))
            .unwrap();
        let expected_bytes = writer.current_offset();
        let wal_path = writer.path().to_path_buf();
        drop(writer);

        // 1. Arquivo limpo
        let (count, bytes) = WalReader::verify_file(&wal_path).unwrap();
        assert_eq!(count, 2);
        assert_eq!(bytes, expected_bytes);

        // 2. Corrompe o arquivo
        {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&wal_path)
                .unwrap();
            file.seek(SeekFrom::Start(2)).unwrap();
            file.write_all(&[0xEE]).unwrap();
        }

        let verify_res = WalReader::verify_file(&wal_path);
        assert!(verify_res.is_err());
    }
}
