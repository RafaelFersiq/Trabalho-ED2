//! Interface de Linha de Comando (CLI) para o Adaptive Storage Engine.
//!
//! Implementa os comandos exigidos pela especificação oficial da disciplina:
//! - `init --data-dir <DIR>`: Inicializa o diretório e metadados.
//! - `run --data-dir <DIR> --input <IN.jsonl> --output <OUT.jsonl>`: Executa operações em lote.
//! - `verify --data-dir <DIR>`: Audita a integridade física de todos os registros persistidos.
//! - `describe`: Exibe metadados, identificação da equipe e recursos suportados.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::engine::StorageEngine;
use crate::error::{EngineError, Result};
use crate::protocol::{process_workload, WorkloadStats};
use crate::wal::{WalReader, WAL_FILE_NAME};

/// Nome padrão do arquivo de metadados gravado no diretório de dados.
pub const METADATA_FILE_NAME: &str = "metadata.json";

/// Definição principal da CLI via `clap`.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "engine",
    about = "Adaptive Storage Engine (LSM-Tree) - Estruturas de Dados 2 (ED2)",
    version,
    author
)]
pub struct Cli {
    /// Subcomando a ser executado
    #[command(subcommand)]
    pub command: Commands,
}

/// Subcomandos suportados pelo storage engine.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Commands {
    /// Inicializa o diretório de dados com os arquivos e metadados necessários.
    Init {
        /// Caminho do diretório de dados
        #[arg(short, long, value_name = "DIR")]
        data_dir: PathBuf,
    },
    /// Executa um lote de operações do workload via protocolo JSON Lines (JSONL).
    Run {
        /// Caminho do diretório de dados
        #[arg(short, long, value_name = "DIR")]
        data_dir: PathBuf,
        /// Arquivo JSONL de entrada contendo as operações
        #[arg(short, long, value_name = "IN.jsonl")]
        input: PathBuf,
        /// Arquivo JSONL de saída para gravação das respostas
        #[arg(short, long, value_name = "OUT.jsonl")]
        output: PathBuf,
    },
    /// Executa verificação completa de integridade física dos dados persistidos.
    Verify {
        /// Caminho do diretório de dados
        #[arg(short, long, value_name = "DIR")]
        data_dir: PathBuf,
    },
    /// Exibe informações da equipe, versão e recursos implementados pelo engine.
    Describe,
}

/// Estrutura de metadados persistida no arquivo `metadata.json` na inicialização.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineMetadata {
    /// Nome identificador do engine
    pub engine: String,
    /// Versão do binário
    pub version: String,
    /// Versão de formato do armazenamento
    pub format_version: u32,
    /// Etapa corrente do projeto
    pub stage: u32,
}

impl Default for EngineMetadata {
    fn default() -> Self {
        Self {
            engine: "Adaptive Storage Engine (LSM-Tree)".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            format_version: 1,
            stage: 1,
        }
    }
}

/// Relatório de inicialização gerado pelo comando `init`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitReport {
    /// Caminho do diretório inicializado
    pub data_dir: PathBuf,
    /// Caminho do arquivo de metadados gerado ou validado
    pub metadata_path: PathBuf,
    /// Indica se o diretório foi criado na execução
    pub created_dir: bool,
    /// Indica se o arquivo de metadados foi recém-criado
    pub created_metadata: bool,
}

/// Relatório de verificação de integridade gerado pelo comando `verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Caminho do diretório de dados auditado
    pub data_dir: PathBuf,
    /// Caminho do arquivo WAL auditado
    pub wal_path: PathBuf,
    /// Quantidade de registros íntegros validados
    pub records_count: usize,
    /// Total de bytes auditados com sucesso
    pub bytes_validated: u64,
    /// Indica se o armazenamento está 100% íntegro
    pub is_valid: bool,
}

/// Relatório descritivo da aplicação gerado pelo comando `describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeReport {
    /// Nome do projeto
    pub project: String,
    /// Versão atual do executável
    pub version: String,
    /// Autores do projeto
    pub authors: Vec<String>,
    /// Número da etapa corrente
    pub stage: u32,
    /// Nome da etapa corrente
    pub stage_name: String,
    /// Descrição sumária
    pub description: String,
    /// Operações suportadas no protocolo JSONL nesta etapa
    pub supported_operations: Vec<String>,
    /// Recursos e diferenciais arquiteturais implementados
    pub features: Vec<String>,
}

impl Default for DescribeReport {
    fn default() -> Self {
        Self {
            project: "Adaptive Storage Engine (ED2)".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            authors: vec!["Equipe ED2".to_string()],
            stage: 1,
            stage_name: "Persistent Storage Engine".to_string(),
            description: "Mecanismo de armazenamento persistente chave-valor em disco para ED2".to_string(),
            supported_operations: vec![
                "put".to_string(),
                "get".to_string(),
                "delete".to_string(),
            ],
            features: vec![
                "Write-Ahead Log (WAL) Append-Only".to_string(),
                "Checksums CRC32 validados com aceleração SIMD (crc32fast)".to_string(),
                "Índice em memória Bitcask-style para leituras O(1)".to_string(),
                "Tolerância a Crash com truncamento seguro de registros parciais".to_string(),
                "Protocolo JSON Lines (JSONL) com preservação estrita de identificador 'id'".to_string(),
                "Garantia de durabilidade física com fsync por operação confirmada".to_string(),
            ],
        }
    }
}

/// Executa a rotina do comando `init`: cria diretório, persiste metadados e prepara o WAL.
pub fn handle_init<P: AsRef<Path>>(data_dir: P) -> Result<InitReport> {
    let data_dir = data_dir.as_ref().to_path_buf();
    let created_dir = if !data_dir.exists() {
        fs::create_dir_all(&data_dir)?;
        true
    } else {
        false
    };

    let metadata_path = data_dir.join(METADATA_FILE_NAME);
    let created_metadata = if !metadata_path.exists() {
        let meta = EngineMetadata::default();
        let json = serde_json::to_string_pretty(&meta).map_err(EngineError::Json)?;
        fs::write(&metadata_path, json)?;
        true
    } else {
        // Valida se o arquivo de metadados existente é legível
        let content = fs::read_to_string(&metadata_path)?;
        let _ = serde_json::from_str::<EngineMetadata>(&content).map_err(EngineError::Json)?;
        false
    };

    // Abre o StorageEngine para garantir que o WAL esteja inicializado e consistente
    let _ = StorageEngine::open(&data_dir)?;

    Ok(InitReport {
        data_dir,
        metadata_path,
        created_dir,
        created_metadata,
    })
}

/// Executa a rotina do comando `run`: processa um workload JSONL com buffers em streaming.
pub fn handle_run<P1: AsRef<Path>, P2: AsRef<Path>, P3: AsRef<Path>>(
    data_dir: P1,
    input: P2,
    output: P3,
) -> Result<WorkloadStats> {
    let data_dir = data_dir.as_ref();
    let input_path = input.as_ref();
    let output_path = output.as_ref();

    if !input_path.exists() {
        return Err(EngineError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Arquivo de entrada não encontrado: {}",
                input_path.display()
            ),
        )));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let file_in = File::open(input_path)?;
    let reader = BufReader::new(file_in);

    let file_out = File::create(output_path)?;
    let mut writer = BufWriter::new(file_out);

    let mut engine = StorageEngine::open(data_dir)?;
    let stats = process_workload(&mut engine, reader, &mut writer)?;

    writer.flush()?;
    engine.sync()?;

    Ok(stats)
}

/// Executa a rotina do comando `verify`: valida todos os registros gravados no log em disco.
pub fn handle_verify<P: AsRef<Path>>(data_dir: P) -> Result<VerifyReport> {
    let data_dir = data_dir.as_ref().to_path_buf();

    if !data_dir.exists() {
        return Err(EngineError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Diretório de dados não encontrado: {}", data_dir.display()),
        )));
    }

    let metadata_path = data_dir.join(METADATA_FILE_NAME);
    if metadata_path.exists() {
        let content = fs::read_to_string(&metadata_path)?;
        let _ = serde_json::from_str::<EngineMetadata>(&content).map_err(EngineError::Json)?;
    }

    let wal_path = data_dir.join(WAL_FILE_NAME);
    let (records_count, bytes_validated) = if wal_path.exists() {
        WalReader::verify_file(&wal_path)?
    } else {
        (0, 0)
    };

    Ok(VerifyReport {
        data_dir,
        wal_path,
        records_count,
        bytes_validated,
        is_valid: true,
    })
}

/// Executa a rotina do comando `describe`: retorna a estrutura descritiva do engine.
pub fn handle_describe() -> DescribeReport {
    DescribeReport::default()
}

/// Despacha a execução do subcomando contido na estrutura `Cli`.
pub fn execute_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { data_dir } => {
            let report = handle_init(&data_dir)?;
            println!(
                "Diretório de dados inicializado com sucesso em '{}'.",
                report.data_dir.display()
            );
        }
        Commands::Run {
            data_dir,
            input,
            output,
        } => {
            let stats = handle_run(&data_dir, &input, &output)?;
            eprintln!(
                "Workload concluído: {} operações processadas (ok: {}, not_found: {}, error: {}).",
                stats.total_processed, stats.ok_count, stats.not_found_count, stats.error_count
            );
        }
        Commands::Verify { data_dir } => {
            let report = handle_verify(&data_dir)?;
            println!(
                "Integridade verificada com sucesso: {} registros íntegros ({} bytes auditados) em '{}'.",
                report.records_count,
                report.bytes_validated,
                report.wal_path.display()
            );
        }
        Commands::Describe => {
            let report = handle_describe();
            let json = serde_json::to_string_pretty(&report).map_err(EngineError::Json)?;
            println!("{json}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_cli_parsing_init() {
        let args = ["engine", "init", "--data-dir", "/tmp/teste_data"];
        let cli = Cli::try_parse_from(args).expect("Parsing init falhou");
        assert_eq!(
            cli.command,
            Commands::Init {
                data_dir: PathBuf::from("/tmp/teste_data")
            }
        );
    }

    #[test]
    fn test_cli_parsing_run() {
        let args = [
            "engine",
            "run",
            "--data-dir",
            "/tmp/teste_data",
            "--input",
            "/tmp/in.jsonl",
            "--output",
            "/tmp/out.jsonl",
        ];
        let cli = Cli::try_parse_from(args).expect("Parsing run falhou");
        assert_eq!(
            cli.command,
            Commands::Run {
                data_dir: PathBuf::from("/tmp/teste_data"),
                input: PathBuf::from("/tmp/in.jsonl"),
                output: PathBuf::from("/tmp/out.jsonl"),
            }
        );
    }

    #[test]
    fn test_cli_parsing_verify() {
        let args = ["engine", "verify", "--data-dir", "/tmp/teste_data"];
        let cli = Cli::try_parse_from(args).expect("Parsing verify falhou");
        assert_eq!(
            cli.command,
            Commands::Verify {
                data_dir: PathBuf::from("/tmp/teste_data")
            }
        );
    }

    #[test]
    fn test_cli_parsing_describe() {
        let args = ["engine", "describe"];
        let cli = Cli::try_parse_from(args).expect("Parsing describe falhou");
        assert_eq!(cli.command, Commands::Describe);
    }

    #[test]
    fn test_handle_init_creates_dir_and_metadata() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("storage_data");

        assert!(!data_dir.exists());
        let report = handle_init(&data_dir).expect("handle_init falhou");

        assert!(data_dir.exists());
        assert!(report.created_dir);
        assert!(report.created_metadata);
        assert!(report.metadata_path.exists());
        assert!(data_dir.join(WAL_FILE_NAME).exists());

        // Segunda chamada (idempotência)
        let report2 = handle_init(&data_dir).expect("segunda chamada handle_init falhou");
        assert!(!report2.created_dir);
        assert!(!report2.created_metadata);
    }

    #[test]
    fn test_handle_describe_content() {
        let report = handle_describe();
        assert_eq!(report.stage, 1);
        assert!(report.supported_operations.contains(&"put".to_string()));
        assert!(report.supported_operations.contains(&"get".to_string()));
        assert!(report.supported_operations.contains(&"delete".to_string()));
        assert!(!report.features.is_empty());

        let json = serde_json::to_string(&report).expect("Serialização falhou");
        assert!(json.contains("stage"));
        assert!(json.contains("supported_operations"));
    }

    #[test]
    fn test_handle_run_and_verify_e2e() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let input_file = dir.path().join("workload.jsonl");
        let output_file = dir.path().join("results.jsonl");

        // Cria dados de entrada JSONL
        let input_data = r#"{"id": 1, "op": "put", "key": 100, "value": "val100"}
{"id": 2, "op": "get", "key": 100}
{"id": 3, "op": "delete", "key": 100}
{"id": 4, "op": "get", "key": 100}
"#;
        fs::write(&input_file, input_data).unwrap();

        // Executa o workload via CLI handle_run
        let stats = handle_run(&data_dir, &input_file, &output_file).expect("handle_run falhou");
        assert_eq!(stats.total_processed, 4);
        assert_eq!(stats.ok_count, 3);
        assert_eq!(stats.not_found_count, 1);
        assert_eq!(stats.error_count, 0);

        // Confere saída gerada
        let out_content = fs::read_to_string(&output_file).unwrap();
        let lines: Vec<&str> = out_content.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], r#"{"id":1,"status":"ok"}"#);
        assert_eq!(lines[1], r#"{"id":2,"status":"ok","value":"val100"}"#);
        assert_eq!(lines[2], r#"{"id":3,"status":"ok"}"#);
        assert_eq!(lines[3], r#"{"id":4,"status":"not_found"}"#);

        // Executa verify nos dados gravados
        let verify_report = handle_verify(&data_dir).expect("handle_verify falhou");
        assert!(verify_report.is_valid);
        // Teve 1 PUT e 1 DELETE -> 2 registros no WAL
        assert_eq!(verify_report.records_count, 2);
        assert!(verify_report.bytes_validated > 0);
    }

    #[test]
    fn test_handle_verify_detects_corrupted_wal() {
        use std::io::Seek;

        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");

        // Inicializa e grava um registro
        let mut engine = StorageEngine::open(&data_dir).unwrap();
        engine.put(1, b"hello".to_vec()).unwrap();
        drop(engine);

        // Corrompe bytes no meio do arquivo WAL
        let wal_path = data_dir.join(WAL_FILE_NAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&wal_path)
            .unwrap();
        // Altera byte do payload para invalidar o CRC32
        file.seek(std::io::SeekFrom::Start(10)).unwrap();
        file.write_all(b"X").unwrap();
        drop(file);

        // Verify deve acusar erro
        let result = handle_verify(&data_dir);
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::CrcMismatch { .. } => {}
            other => panic!("Esperava CrcMismatch, recebeu: {other:?}"),
        }
    }

    #[test]
    fn test_execute_cli_subcommands() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("cli_test_dir");

        // Executa Init via execute_cli
        let cli_init = Cli {
            command: Commands::Init {
                data_dir: data_dir.clone(),
            },
        };
        execute_cli(cli_init).expect("execute_cli Init falhou");

        // Executa Verify via execute_cli
        let cli_verify = Cli {
            command: Commands::Verify {
                data_dir: data_dir.clone(),
            },
        };
        execute_cli(cli_verify).expect("execute_cli Verify falhou");

        // Executa Describe via execute_cli
        let cli_desc = Cli {
            command: Commands::Describe,
        };
        execute_cli(cli_desc).expect("execute_cli Describe falhou");
    }
}
