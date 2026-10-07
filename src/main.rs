//! Ponto de Entrada Principal (src/main.rs) do Adaptive Storage Engine.
//!
//! Responsável por inicializar a aplicação, realizar o parsing dos argumentos
//! da linha de comando com `clap`, despachar o subcomando apropriado para o
//! módulo `cli` e efetuar o tratamento global de erros com encerramento padronizado.

use std::error::Error;
use std::process;

use clap::Parser;
use storage_engine::cli::{execute_cli, Cli};
use storage_engine::error::Result;

/// Formata e exibe mensagens de erro com sua cadeia de causas no `stderr`.
pub fn print_error(err: &(dyn Error + 'static)) {
    eprintln!("Erro: {err}");
    let mut source = err.source();
    while let Some(cause) = source {
        eprintln!("  Causa: {cause}");
        source = cause.source();
    }
}

/// Executa a aplicação realizando o parsing dos argumentos do ambiente
/// e despachando a execução para a CLI.
pub fn run_app() -> Result<()> {
    let cli = Cli::parse();
    execute_cli(cli)
}

fn main() {
    if let Err(err) = run_app() {
        print_error(&err);
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_print_error_formatting() {
        use storage_engine::error::EngineError;
        let err = EngineError::CrcMismatch {
            expected: 0x1234,
            calculated: 0x5678,
            offset: 42,
        };
        // Garante que print_error não cause pânico
        print_error(&err);
    }

    #[test]
    fn test_print_error_with_source() {
        use storage_engine::error::EngineError;
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "arquivo ausente");
        let err = EngineError::Io(io_err);
        assert!(err.source().is_some());
        print_error(&err);
    }

    #[test]
    fn test_dispatch_describe() {
        let cli = Cli::try_parse_from(["engine", "describe"]).expect("Parse describe");
        let result = execute_cli(cli);
        assert!(result.is_ok());
    }

    #[test]
    fn test_dispatch_init_and_verify_success() {
        let dir = tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap();

        // 1. Executa init
        let cli_init = Cli::try_parse_from(["engine", "init", "--data-dir", path_str])
            .expect("Parse init");
        let res_init = execute_cli(cli_init);
        assert!(res_init.is_ok());

        // 2. Executa verify no diretório inicializado
        let cli_verify = Cli::try_parse_from(["engine", "verify", "--data-dir", path_str])
            .expect("Parse verify");
        let res_verify = execute_cli(cli_verify);
        assert!(res_verify.is_ok());
    }

    #[test]
    fn test_dispatch_run_workload() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let input_path = dir.path().join("input.jsonl");
        let output_path = dir.path().join("output.jsonl");

        // Cria arquivo de workload JSONL
        {
            let mut f = File::create(&input_path).unwrap();
            writeln!(f, r#"{{"id": 1, "op": "put", "key": 42, "value": "val42"}}"#).unwrap();
            writeln!(f, r#"{{"id": 2, "op": "get", "key": 42}}"#).unwrap();
        }

        let cli_run = Cli::try_parse_from([
            "engine",
            "run",
            "--data-dir",
            data_dir.to_str().unwrap(),
            "--input",
            input_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .expect("Parse run");

        let res_run = execute_cli(cli_run);
        assert!(res_run.is_ok());
        assert!(output_path.exists());
    }

    #[test]
    fn test_dispatch_verify_error_on_nonexistent_dir() {
        let cli = Cli::try_parse_from([
            "engine",
            "verify",
            "--data-dir",
            "/caminho/com_certeza_inexistente_987654321",
        ])
        .expect("Parse verify");

        let res = execute_cli(cli);
        assert!(res.is_err());
    }
}
