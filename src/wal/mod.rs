//! Módulo de gerenciamento do Write-Ahead Log (WAL) e arquivos de dados binários.

pub mod reader;
pub mod record;
pub mod writer;

pub use reader::{RecoveryReport, WalEntry, WalIterator, WalReader};
pub use record::{calculate_crc, LogRecord, FLAG_DELETE, FLAG_PUT, HEADER_SIZE, MAX_VALUE_LEN};
pub use writer::{WalWriter, WAL_FILE_NAME};
