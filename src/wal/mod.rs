//! Módulo de gerenciamento do Write-Ahead Log (WAL) e arquivos de dados binários.

pub mod record;

pub use record::{calculate_crc, LogRecord, FLAG_DELETE, FLAG_PUT, HEADER_SIZE, MAX_VALUE_LEN};
