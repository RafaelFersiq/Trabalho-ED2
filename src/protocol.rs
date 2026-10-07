//! Módulo de protocolo de comunicação JSON Lines (JSONL).
//!
//! Implementa os formatos padronizados de mensagens de entrada (`Request`) e saída (`Response`),
//! garantindo a preservação estrita do campo `id` em todas as respostas geradas.

use std::io::{BufRead, Write};
use serde::{Deserialize, Serialize};

use crate::engine::StorageEngine;
use crate::error::{EngineError, Key, Result};

/// Representação de uma requisição de operação vinda de uma linha JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Identificador único da requisição, obrigatório e preservado na resposta.
    pub id: u64,
    /// Operação a ser executada com seus respectivos parâmetros.
    #[serde(flatten)]
    pub op: Operation,
}

impl Request {
    /// Cria uma nova requisição `PUT`.
    pub fn put(id: u64, key: Key, value: impl Into<String>) -> Self {
        Self {
            id,
            op: Operation::Put {
                key,
                value: value.into(),
            },
        }
    }

    /// Cria uma nova requisição `GET`.
    pub fn get(id: u64, key: Key) -> Self {
        Self {
            id,
            op: Operation::Get { key },
        }
    }

    /// Cria uma nova requisição `DELETE`.
    pub fn delete(id: u64, key: Key) -> Self {
        Self {
            id,
            op: Operation::Delete { key },
        }
    }

    /// Cria uma nova requisição `SCAN`.
    pub fn scan(id: u64, start: Key, end: Key) -> Self {
        Self {
            id,
            op: Operation::Scan { start, end },
        }
    }

    /// Desserializa uma requisição a partir de uma linha de texto JSON.
    pub fn from_json_line(line: &str) -> Result<Self> {
        serde_json::from_str(line.trim()).map_err(EngineError::Json)
    }

    /// Serializa a requisição para uma string no formato JSON (sem quebra de linha final).
    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string(self).map_err(EngineError::Json)
    }
}

/// Variantes de operações suportadas pelo storage engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Inserção ou atualização de valor associado a uma chave inteira `u64`.
    Put {
        /// Chave de 64 bits sem sinal.
        key: Key,
        /// Conteúdo textual do valor.
        value: String,
    },
    /// Consulta pontual por valor associado a uma chave.
    Get {
        /// Chave de 64 bits sem sinal a ser consultada.
        key: Key,
    },
    /// Exclusão lógica de registro associado à chave via tombstone.
    Delete {
        /// Chave de 64 bits sem sinal a ser removida.
        key: Key,
    },
    /// Consulta em intervalo inclusivo de chaves (ordenado crescente).
    Scan {
        /// Chave inicial inclusiva do intervalo.
        start: Key,
        /// Chave final inclusiva do intervalo.
        end: Key,
    },
}

/// Estado de conclusão de uma operação retornada na resposta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    /// Operação realizada com sucesso.
    Ok,
    /// Registro solicitado não foi encontrado ou está excluído logicamente.
    NotFound,
    /// Falha operacional, erro de integridade ou comando inválido/não suportado.
    Error,
}

/// Registro individual retornado no vetor `records` de uma consulta `SCAN`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanRecord {
    /// Chave de 64 bits sem sinal.
    pub key: Key,
    /// Conteúdo do valor recuperado.
    pub value: String,
}

/// Mensagem de resposta padronizada enviada ao cliente em formato JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    /// Identificador correspondente à requisição que originou esta resposta.
    pub id: u64,
    /// Status do resultado da operação.
    pub status: ResponseStatus,
    /// Valor textual retornado (presente apenas em consultas `GET` bem-sucedidas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Lista de registros ordenados retornados (presente em consultas `SCAN`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<Vec<ScanRecord>>,
    /// Mensagem descritiva de erro (presente em respostas com status `error`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Response {
    /// Cria uma resposta de sucesso genérica (`{"id": id, "status": "ok"}`).
    pub fn ok(id: u64) -> Self {
        Self {
            id,
            status: ResponseStatus::Ok,
            value: None,
            records: None,
            message: None,
        }
    }

    /// Cria uma resposta de sucesso com valor para operações `GET` (`{"id": id, "status": "ok", "value": "..."}`).
    pub fn ok_with_value(id: u64, value: impl Into<String>) -> Self {
        Self {
            id,
            status: ResponseStatus::Ok,
            value: Some(value.into()),
            records: None,
            message: None,
        }
    }

    /// Cria uma resposta de sucesso com vetor de registros para operações `SCAN`.
    pub fn ok_with_records(id: u64, records: Vec<ScanRecord>) -> Self {
        Self {
            id,
            status: ResponseStatus::Ok,
            value: None,
            records: Some(records),
            message: None,
        }
    }

    /// Cria uma resposta para chave inexistente (`{"id": id, "status": "not_found"}`).
    pub fn not_found(id: u64) -> Self {
        Self {
            id,
            status: ResponseStatus::NotFound,
            value: None,
            records: None,
            message: None,
        }
    }

    /// Cria uma resposta de erro (`{"id": id, "status": "error", "message": "..."}`).
    pub fn error(id: u64, message: impl Into<String>) -> Self {
        Self {
            id,
            status: ResponseStatus::Error,
            value: None,
            records: None,
            message: Some(message.into()),
        }
    }

    /// Serializa a resposta para uma linha de texto JSON (sem quebra de linha).
    pub fn to_json_string(&self) -> Result<String> {
        serde_json::to_string(self).map_err(EngineError::Json)
    }

    /// Escreve a resposta serializada no buffer de escrita fornecido, seguida por `\n`.
    pub fn write_json_line<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self).map_err(EngineError::Json)?;
        writer.write_all(b"\n")?;
        Ok(())
    }
}

/// Executa uma única requisição contra o `StorageEngine` preservando estritamente o `id`.
pub fn execute_request(engine: &mut StorageEngine, request: Request) -> Response {
    let req_id = request.id;
    match request.op {
        Operation::Put { key, value } => match engine.put(key, value.into_bytes()) {
            Ok(_) => Response::ok(req_id),
            Err(err) => Response::error(req_id, err.to_string()),
        },
        Operation::Get { key } => match engine.get(key) {
            Ok(Some(raw_bytes)) => match String::from_utf8(raw_bytes) {
                Ok(val_str) => Response::ok_with_value(req_id, val_str),
                Err(err) => Response::error(
                    req_id,
                    format!("Payload recuperado não é UTF-8 válido: {err}"),
                ),
            },
            Ok(None) => Response::not_found(req_id),
            Err(err) => Response::error(req_id, err.to_string()),
        },
        Operation::Delete { key } => match engine.delete(key) {
            Ok(_) => Response::ok(req_id),
            Err(err) => Response::error(req_id, err.to_string()),
        },
        Operation::Scan { .. } => Response::error(
            req_id,
            "Operação 'scan' não é suportada na Etapa 1 (escopo funcional da Etapa 2)",
        ),
    }
}

/// Estatísticas de processamento de um lote de workload JSONL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkloadStats {
    /// Total de linhas válidas processadas.
    pub total_processed: usize,
    /// Quantidade de operações com status `ok`.
    pub ok_count: usize,
    /// Quantidade de operações com status `not_found`.
    pub not_found_count: usize,
    /// Quantidade de operações com status `error`.
    pub error_count: usize,
}

/// Processa um fluxo contínuo de requisições JSONL de entrada, escrevendo as respostas em streaming.
///
/// # Eficiência de I/O:
/// Utiliza `BufRead` e `Write` para ler e descarregar linha a linha sem carregar o lote
/// inteiro na memória RAM, garantindo conformidade com os limites rígidos de memória do projeto.
pub fn process_workload<R: BufRead, W: Write>(
    engine: &mut StorageEngine,
    reader: R,
    mut writer: W,
) -> Result<WorkloadStats> {
    let mut stats = WorkloadStats::default();

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match Request::from_json_line(trimmed) {
            Ok(request) => execute_request(engine, request),
            Err(err) => {
                // Tenta extrair um campo "id" da linha malformada via JSON genérico para não perder o id
                let fallback_id = serde_json::from_str::<serde_json::Value>(trimmed)
                    .ok()
                    .and_then(|val| val.get("id").and_then(|id_val| id_val.as_u64()))
                    .unwrap_or(0);

                Response::error(
                    fallback_id,
                    format!("Erro ao interpretar requisição JSONL: {err}"),
                )
            }
        };

        match response.status {
            ResponseStatus::Ok => stats.ok_count += 1,
            ResponseStatus::NotFound => stats.not_found_count += 1,
            ResponseStatus::Error => stats.error_count += 1,
        }
        stats.total_processed += 1;

        response.write_json_line(&mut writer)?;
    }

    writer.flush()?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    #[test]
    fn test_deserialize_requests() {
        // 1. PUT
        let json_put = r#"{"id": 1, "op": "put", "key": 91, "value": "abc"}"#;
        let req_put = Request::from_json_line(json_put).unwrap();
        assert_eq!(req_put.id, 1);
        assert_eq!(
            req_put.op,
            Operation::Put {
                key: 91,
                value: "abc".into()
            }
        );

        // 2. GET
        let json_get = r#"{"id": 2, "op": "get", "key": 91}"#;
        let req_get = Request::from_json_line(json_get).unwrap();
        assert_eq!(req_get.id, 2);
        assert_eq!(req_get.op, Operation::Get { key: 91 });

        // 3. DELETE
        let json_delete = r#"{"id": 3, "op": "delete", "key": 91}"#;
        let req_delete = Request::from_json_line(json_delete).unwrap();
        assert_eq!(req_delete.id, 3);
        assert_eq!(req_delete.op, Operation::Delete { key: 91 });

        // 4. SCAN
        let json_scan = r#"{"id": 4, "op": "scan", "start": 0, "end": 100}"#;
        let req_scan = Request::from_json_line(json_scan).unwrap();
        assert_eq!(req_scan.id, 4);
        assert_eq!(req_scan.op, Operation::Scan { start: 0, end: 100 });
    }

    #[test]
    fn test_serialize_responses() {
        // 1. Resposta PUT / DELETE OK
        let resp_ok = Response::ok(1);
        let json_ok = resp_ok.to_json_string().unwrap();
        assert_eq!(json_ok, r#"{"id":1,"status":"ok"}"#);

        // 2. Resposta GET OK com valor
        let resp_val = Response::ok_with_value(2, "abc");
        let json_val = resp_val.to_json_string().unwrap();
        assert_eq!(json_val, r#"{"id":2,"status":"ok","value":"abc"}"#);

        // 3. Resposta NOT_FOUND
        let resp_nf = Response::not_found(10);
        let json_nf = resp_nf.to_json_string().unwrap();
        assert_eq!(json_nf, r#"{"id":10,"status":"not_found"}"#);

        // 4. Resposta SCAN com records
        let resp_scan = Response::ok_with_records(
            4,
            vec![ScanRecord {
                key: 91,
                value: "abc".into(),
            }],
        );
        let json_scan = resp_scan.to_json_string().unwrap();
        assert_eq!(
            json_scan,
            r#"{"id":4,"status":"ok","records":[{"key":91,"value":"abc"}]}"#
        );
    }

    #[test]
    fn test_execute_workload_roundtrip() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        let input_jsonl = "\
{\"id\": 1, \"op\": \"put\", \"key\": 91, \"value\": \"abc\"}\n\
{\"id\": 2, \"op\": \"get\", \"key\": 91}\n\
{\"id\": 3, \"op\": \"delete\", \"key\": 91}\n\
{\"id\": 4, \"op\": \"get\", \"key\": 91}\n\
{\"id\": 5, \"op\": \"scan\", \"start\": 0, \"end\": 100}\n";

        let reader = Cursor::new(input_jsonl.as_bytes());
        let mut output = Vec::new();

        let stats = process_workload(&mut engine, reader, &mut output).unwrap();

        assert_eq!(stats.total_processed, 5);
        assert_eq!(stats.ok_count, 3); // 1 (put), 2 (get), 3 (delete)
        assert_eq!(stats.not_found_count, 1); // 4 (get após delete)
        assert_eq!(stats.error_count, 1); // 5 (scan não suportado na Etapa 1)

        let output_str = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = output_str.lines().collect();
        assert_eq!(lines.len(), 5);

        assert_eq!(lines[0], r#"{"id":1,"status":"ok"}"#);
        assert_eq!(lines[1], r#"{"id":2,"status":"ok","value":"abc"}"#);
        assert_eq!(lines[2], r#"{"id":3,"status":"ok"}"#);
        assert_eq!(lines[3], r#"{"id":4,"status":"not_found"}"#);

        // Linha 5 deve ter id=5 e status="error"
        let resp5: serde_json::Value = serde_json::from_str(lines[4]).unwrap();
        assert_eq!(resp5["id"], 5);
        assert_eq!(resp5["status"], "error");
    }

    #[test]
    fn test_id_preservation_strict() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        let input_jsonl = "\
{\"id\": 999999, \"op\": \"put\", \"key\": 10, \"value\": \"v1\"}\n\
{\"id\": 123456, \"op\": \"get\", \"key\": 10}\n\
{\"id\": 777888, \"op\": \"delete\", \"key\": 10}\n";

        let reader = Cursor::new(input_jsonl.as_bytes());
        let mut output = Vec::new();

        process_workload(&mut engine, reader, &mut output).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = output_str.lines().collect();

        let val0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(val0["id"], 999999);

        let val1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(val1["id"], 123456);

        let val2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(val2["id"], 777888);
    }
}
