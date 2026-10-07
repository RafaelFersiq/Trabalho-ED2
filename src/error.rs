//! Módulo central de tratamento de erros e tipos básicos do storage engine.

use std::fmt;
use std::io;

/// Tipo fundamental para chaves no storage engine (inteiro sem sinal de 64 bits).
pub type Key = u64;

/// Apontador em memória para a localização física de um registro no log em disco.
/// Usado pelo índice estilo Bitcask na Etapa 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordLocation {
    /// Deslocamento em bytes a partir do início do arquivo de log onde o registro começa.
    pub offset: u64,
    /// Tamanho em bytes do payload do valor.
    pub value_len: u32,
}

impl RecordLocation {
    /// Cria uma nova instância de `RecordLocation`.
    pub const fn new(offset: u64, value_len: u32) -> Self {
        Self { offset, value_len }
    }
}

/// Enumeração de todos os erros operacionais e de integridade do storage engine.
#[derive(Debug)]
pub enum EngineError {
    /// Erro de entrada/saída retornado pelo sistema operacional.
    Io(io::Error),

    /// Falha de integridade: o CRC32 calculado não coincide com o CRC32 gravado no registro.
    CrcMismatch {
        /// CRC32 esperado (armazenado no cabeçalho do registro).
        expected: u32,
        /// CRC32 calculado sobre os bytes lidos do disco.
        calculated: u32,
        /// Offset do início do registro no arquivo onde a divergência ocorreu.
        offset: u64,
    },

    /// Fim de arquivo inesperado durante a leitura de um registro (possível crash durante gravação).
    UnexpectedEof,

    /// Formato de registro binário inválido (flags desconhecidas, comprimentos incompatíveis, etc.).
    InvalidRecord(String),

    /// Erro de serialização ou deserialização no formato JSON Lines.
    Json(serde_json::Error),

    /// Inconsistência de metadados ou corrupção lógica do diretório de dados.
    Corruption(String),
}

/// Tipo alias de conveniência para resultados que utilizam `EngineError`.
pub type Result<T> = std::result::Result<T, EngineError>;

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "Erro de I/O no disco: {err}"),
            Self::CrcMismatch {
                expected,
                calculated,
                offset,
            } => write!(
                f,
                "Corrupção de integridade no offset {offset}: CRC32 esperado {expected:#010x}, calculado {calculated:#010x}"
            ),
            Self::UnexpectedEof => {
                write!(f, "Fim de arquivo inesperado ao ler registro (gravação incompleta)")
            }
            Self::InvalidRecord(msg) => write!(f, "Registro binário inválido: {msg}"),
            Self::Json(err) => write!(f, "Erro de processamento JSONL: {err}"),
            Self::Corruption(msg) => write!(f, "Corrupção ou inconsistência detectada: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Json(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for EngineError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for EngineError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_location() {
        let loc = RecordLocation::new(128, 42);
        assert_eq!(loc.offset, 128);
        assert_eq!(loc.value_len, 42);
        assert_eq!(loc, RecordLocation { offset: 128, value_len: 42 });
    }

    #[test]
    fn test_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "arquivo inexistente");
        let engine_err: EngineError = io_err.into();
        match engine_err {
            EngineError::Io(err) => assert_eq!(err.kind(), io::ErrorKind::NotFound),
            _ => panic!("Esperado EngineError::Io"),
        }
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("{invalido").unwrap_err();
        let engine_err: EngineError = json_err.into();
        match engine_err {
            EngineError::Json(_) => (),
            _ => panic!("Esperado EngineError::Json"),
        }
    }

    #[test]
    fn test_display_messages() {
        let err_crc = EngineError::CrcMismatch {
            expected: 0x12345678,
            calculated: 0x87654321,
            offset: 1024,
        };
        let msg = format!("{err_crc}");
        assert!(msg.contains("offset 1024"));
        assert!(msg.contains("0x12345678"));
        assert!(msg.contains("0x87654321"));

        let err_eof = EngineError::UnexpectedEof;
        assert_eq!(
            format!("{err_eof}"),
            "Fim de arquivo inesperado ao ler registro (gravação incompleta)"
        );

        let err_rec = EngineError::InvalidRecord("flag 0xFF desconhecida".into());
        assert_eq!(
            format!("{err_rec}"),
            "Registro binário inválido: flag 0xFF desconhecida"
        );

        let err_corr = EngineError::Corruption("diretório corrompido".into());
        assert_eq!(
            format!("{err_corr}"),
            "Corrupção ou inconsistência detectada: diretório corrompido"
        );
    }

    #[test]
    fn test_question_mark_operator() {
        fn fail_io() -> Result<()> {
            let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "sem permissao");
            Err(io_err)?
        }

        let res = fail_io();
        assert!(res.is_err());
        match res.unwrap_err() {
            EngineError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
            _ => panic!("Esperado erro de I/O"),
        }
    }
}
