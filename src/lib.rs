//! Adaptive Storage Engine (LSM-Tree)
//! Disciplina de Estruturas de Dados 2 (ED2)

pub mod cli;
pub mod engine;
pub mod error;
pub mod protocol;
pub mod wal;

pub use cli::{
    execute_cli, handle_describe, handle_init, handle_run, handle_verify, Cli, Commands,
    DescribeReport, EngineMetadata, InitReport, VerifyReport, METADATA_FILE_NAME,
};
pub use engine::StorageEngine;
pub use error::{EngineError, Key, RecordLocation, Result};
pub use protocol::{
    execute_request, process_workload, Operation, Request, Response, ResponseStatus, ScanRecord,
    WorkloadStats,
};
pub use wal::{
    calculate_crc, LogRecord, RecoveryReport, WalEntry, WalIterator, WalReader, WalWriter,
    FLAG_DELETE, FLAG_PUT, HEADER_SIZE, WAL_FILE_NAME,
};

