//! Suíte de Testes Automatizados da Etapa 1: Ponta a Ponta com Workload JSON Lines (JSONL).
//!
//! Valida a execução em lote do comando CLI `run`, a comunicação via streaming
//! com `BufReader`/`BufWriter`, a preservação rígida do identificador `id` em todas
//! as respostas e o ciclo de vida completo de `put`, `get`, `delete` e `scan`.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use storage_engine::cli::{execute_cli, handle_run, handle_verify, Cli, Commands};
use storage_engine::protocol::{Request, Response, ResponseStatus};
use tempfile::tempdir;

#[test]
fn test_e2e_jsonl_lifecycle_put_get_delete() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let input_file = dir.path().join("workload_input.jsonl");
    let output_file = dir.path().join("workload_output.jsonl");

    // Prepara arquivo JSONL com ciclo completo
    let requests = vec![
        Request::put(1, 10, "valor_10"),
        Request::put(2, 20, "valor_20"),
        Request::get(3, 10),
        Request::get(4, 99), // Inexistente
        Request::put(5, 10, "valor_10_atualizado"),
        Request::get(6, 10),
        Request::delete(7, 20),
        Request::get(8, 20), // Excluído logicamente
        Request::delete(9, 999), // Delete de inexistente
    ];

    {
        let mut file = File::create(&input_file).unwrap();
        for req in &requests {
            writeln!(file, "{}", req.to_json_string().unwrap()).unwrap();
        }
    }

    // Executa o comando `run`
    let stats = handle_run(&data_dir, &input_file, &output_file)
        .expect("handle_run deve executar com sucesso");

    assert_eq!(stats.total_processed, 9);
    assert_eq!(stats.ok_count, 7);
    assert_eq!(stats.not_found_count, 2);
    assert_eq!(stats.error_count, 0);

    // Lê e valida o arquivo de saída JSONL gerado
    let out_file = File::open(&output_file).unwrap();
    let lines: Vec<String> = BufReader::new(out_file)
        .lines()
        .map(|l| l.unwrap())
        .collect();

    assert_eq!(lines.len(), 9);

    let responses: Vec<Response> = lines
        .iter()
        .map(|l| serde_json::from_str(l).expect("Linha de saída deve ser JSON válido"))
        .collect();

    // 1. put 10 -> ok
    assert_eq!(responses[0].id, 1);
    assert_eq!(responses[0].status, ResponseStatus::Ok);
    assert_eq!(responses[0].value, None);

    // 2. put 20 -> ok
    assert_eq!(responses[1].id, 2);
    assert_eq!(responses[1].status, ResponseStatus::Ok);
    assert_eq!(responses[1].value, None);

    // 3. get 10 -> ok, value = valor_10
    assert_eq!(responses[2].id, 3);
    assert_eq!(responses[2].status, ResponseStatus::Ok);
    assert_eq!(responses[2].value.as_deref(), Some("valor_10"));

    // 4. get 99 -> not_found
    assert_eq!(responses[3].id, 4);
    assert_eq!(responses[3].status, ResponseStatus::NotFound);
    assert_eq!(responses[3].value, None);

    // 5. put 10 atualizado -> ok
    assert_eq!(responses[4].id, 5);
    assert_eq!(responses[4].status, ResponseStatus::Ok);

    // 6. get 10 -> ok, value = valor_10_atualizado
    assert_eq!(responses[5].id, 6);
    assert_eq!(responses[5].status, ResponseStatus::Ok);
    assert_eq!(responses[5].value.as_deref(), Some("valor_10_atualizado"));

    // 7. delete 20 -> ok
    assert_eq!(responses[6].id, 7);
    assert_eq!(responses[6].status, ResponseStatus::Ok);

    // 8. get 20 (deletado) -> not_found
    assert_eq!(responses[7].id, 8);
    assert_eq!(responses[7].status, ResponseStatus::NotFound);
    assert_eq!(responses[7].value, None);

    // 9. delete 999 (inexistente) -> ok
    assert_eq!(responses[8].id, 9);
    assert_eq!(responses[8].status, ResponseStatus::Ok);

    // Auditoria final de integridade com verify
    let verify_rep = handle_verify(&data_dir).expect("Auditoria de verify deve ter sucesso");
    assert!(verify_rep.is_valid);
}

#[test]
fn test_e2e_jsonl_strict_id_preservation_and_arbitrary_ids() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data_id_test");
    let input_file = dir.path().join("input_arbitrary_ids.jsonl");
    let output_file = dir.path().join("output_arbitrary_ids.jsonl");

    // Conjunto de IDs não-sequenciais e de magnitudes distintas
    let arbitrary_ids = vec![
        0u64,
        999_999_999_999_999u64,
        42u64,
        7u64,
        u64::MAX,
        1_000_000u64,
        12345u64,
    ];

    {
        let mut file = File::create(&input_file).unwrap();
        for &id in &arbitrary_ids {
            let req = Request::put(id, id % 100, format!("payload_para_id_{id}"));
            writeln!(file, "{}", req.to_json_string().unwrap()).unwrap();
        }
    }

    handle_run(&data_dir, &input_file, &output_file).unwrap();

    let out_file = File::open(&output_file).unwrap();
    let responses: Vec<Response> = BufReader::new(out_file)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();

    assert_eq!(responses.len(), arbitrary_ids.len());

    for (resp, &expected_id) in responses.iter().zip(arbitrary_ids.iter()) {
        assert_eq!(
            resp.id, expected_id,
            "O campo 'id' deve ser rigorosamente preservado na resposta"
        );
        assert_eq!(resp.status, ResponseStatus::Ok);
    }
}

#[test]
fn test_e2e_jsonl_persistence_across_multiple_run_invocations() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data_multi_run");

    let input1 = dir.path().join("batch1.jsonl");
    let output1 = dir.path().join("out1.jsonl");

    let input2 = dir.path().join("batch2.jsonl");
    let output2 = dir.path().join("out2.jsonl");

    // Lote 1: Insere 50 chaves
    {
        let mut file = File::create(&input1).unwrap();
        for k in 1..=50u64 {
            let req = Request::put(k, k, format!("lote_1_valor_{k}"));
            writeln!(file, "{}", req.to_json_string().unwrap()).unwrap();
        }
    }

    let stats1 = handle_run(&data_dir, &input1, &output1).unwrap();
    assert_eq!(stats1.total_processed, 50);
    assert_eq!(stats1.ok_count, 50);

    // Lote 2: Nova invocação de `run` (reabrindo o engine) para consultar as 50 chaves
    {
        let mut file = File::create(&input2).unwrap();
        for k in 1..=50u64 {
            let req = Request::get(1000 + k, k);
            writeln!(file, "{}", req.to_json_string().unwrap()).unwrap();
        }
    }

    let stats2 = handle_run(&data_dir, &input2, &output2).unwrap();
    assert_eq!(stats2.total_processed, 50);
    assert_eq!(stats2.ok_count, 50);
    assert_eq!(stats2.not_found_count, 0);

    // Valida respostas do lote 2
    let out2 = File::open(&output2).unwrap();
    let responses: Vec<Response> = BufReader::new(out2)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();

    for (idx, resp) in responses.iter().enumerate() {
        let k = (idx + 1) as u64;
        assert_eq!(resp.id, 1000 + k);
        assert_eq!(resp.status, ResponseStatus::Ok);
        assert_eq!(
            resp.value.as_deref(),
            Some(format!("lote_1_valor_{k}").as_str())
        );
    }
}

#[test]
fn test_e2e_jsonl_scan_operation_returns_error_status_in_stage1() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data_scan_test");
    let input_file = dir.path().join("input_scan.jsonl");
    let output_file = dir.path().join("output_scan.jsonl");

    {
        let mut file = File::create(&input_file).unwrap();
        let req = Request::scan(777, 10, 50);
        writeln!(file, "{}", req.to_json_string().unwrap()).unwrap();
    }

    let stats = handle_run(&data_dir, &input_file, &output_file).unwrap();
    assert_eq!(stats.total_processed, 1);
    assert_eq!(stats.error_count, 1);

    let out_file = File::open(&output_file).unwrap();
    let line = BufReader::new(out_file).lines().next().unwrap().unwrap();
    let resp: Response = serde_json::from_str(&line).unwrap();

    assert_eq!(resp.id, 777);
    assert_eq!(resp.status, ResponseStatus::Error);
    assert!(
        resp.message
            .as_deref()
            .unwrap_or("")
            .contains("não é suportada na Etapa 1"),
        "Mensagem de erro de SCAN deve explicar a restrição da Etapa 1"
    );
}

#[test]
fn test_e2e_jsonl_empty_lines_and_whitespace_are_safely_ignored() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data_whitespace");
    let input_file = dir.path().join("input_ws.jsonl");
    let output_file = dir.path().join("output_ws.jsonl");

    {
        let mut file = File::create(&input_file).unwrap();
        writeln!(file, "\n   \n").unwrap(); // Linhas vazias iniciais
        writeln!(file, "{}", Request::put(1, 10, "val1").to_json_string().unwrap()).unwrap();
        writeln!(file, "\n  \t  \n").unwrap(); // Linhas vazias no meio
        writeln!(file, "{}", Request::get(2, 10).to_json_string().unwrap()).unwrap();
        writeln!(file, "\n\n").unwrap(); // Linhas vazias no fim
    }

    let stats = handle_run(&data_dir, &input_file, &output_file).unwrap();
    assert_eq!(stats.total_processed, 2);
    assert_eq!(stats.ok_count, 2);

    let out_file = File::open(&output_file).unwrap();
    let lines: Vec<String> = BufReader::new(out_file)
        .lines()
        .map(|l| l.unwrap())
        .collect();

    assert_eq!(lines.len(), 2);
}

#[test]
fn test_e2e_cli_complete_workflow_init_run_verify_describe() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data_cli_flow");
    let input_file = dir.path().join("cli_in.jsonl");
    let output_file = dir.path().join("cli_out.jsonl");

    // 1. Init
    let cli_init = Cli {
        command: Commands::Init {
            data_dir: data_dir.clone(),
        },
    };
    execute_cli(cli_init).expect("execute_cli(Init) deve funcionar");
    assert!(data_dir.exists());
    assert!(data_dir.join("metadata.json").exists());

    // 2. Run
    {
        let mut file = File::create(&input_file).unwrap();
        writeln!(file, "{}", Request::put(100, 555, "cli_val").to_json_string().unwrap()).unwrap();
        writeln!(file, "{}", Request::get(200, 555).to_json_string().unwrap()).unwrap();
    }

    let cli_run = Cli {
        command: Commands::Run {
            data_dir: data_dir.clone(),
            input: input_file,
            output: output_file.clone(),
        },
    };
    execute_cli(cli_run).expect("execute_cli(Run) deve funcionar");
    assert!(output_file.exists());

    // 3. Verify
    let cli_verify = Cli {
        command: Commands::Verify {
            data_dir: data_dir.clone(),
        },
    };
    execute_cli(cli_verify).expect("execute_cli(Verify) deve funcionar");

    // 4. Describe
    let cli_describe = Cli {
        command: Commands::Describe,
    };
    execute_cli(cli_describe).expect("execute_cli(Describe) deve funcionar");
}
