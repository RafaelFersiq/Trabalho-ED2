//! Adaptive Storage Engine (LSM-Tree)
//! Disciplina de Estruturas de Dados 2 (ED2)

pub mod error;
pub mod wal;

pub use error::{EngineError, Key, RecordLocation, Result};
pub use wal::{calculate_crc, LogRecord, FLAG_DELETE, FLAG_PUT, HEADER_SIZE};
