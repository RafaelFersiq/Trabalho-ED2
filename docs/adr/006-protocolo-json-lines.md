# ADR-006: Protocolo JSON Lines (JSONL) e Despacho de Operações

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

A interface oficial de interação em lote do *Storage Engine* é o protocolo **JSON Lines (JSONL)**, no qual cada linha de entrada representa uma requisição e cada linha de saída representa a resposta correspondente.

Os requisitos mandatórios para o protocolo incluem:
1. **Preservação Rígida do Identificador (`id`):** Toda resposta gerada deve preservar obrigatoriamente o campo `id` da requisição correspondente, permitindo rastreabilidade assíncrona ou concorrente pelo cliente de benchmark.
2. **Representação Estrita dos Tipos de Dados:** Chaves são inteiros sem sinal de 64 bits (`u64`), valores são strings UTF-8 de comprimento variável, e operações são identificadas pelo atributo `op` (`put`, `get`, `delete`, `scan`).
3. **Respostas Estruturadas e Enxutas:** O campo `status` deve ser um dos literais `"ok"`, `"not_found"` ou `"error"`. Campos contextuais (`value`, `records`, `message`) devem ser omitidos na serialização quando ausentes (`skip_serializing_if = "Option::is_none"`).
4. **I/O em Streaming com Baixo Consumo de RAM:** Arquivos de workload podem ter milhões de operações e centenas de megabytes. Carregar todo o arquivo na memória RAM violaria a regra fundamental contra OOM. O processamento deve ser estritamente linha a linha bufferizado.

---

## 2. Decisão

1. **Modelagem de Requisições (`Request` e `Operation` em `src/protocol.rs`):**
   * Struct `Request` com campo `id: u64` e `#[serde(flatten)] pub op: Operation`.
   * Enum `Operation` com `#[serde(tag = "op", rename_all = "snake_case")]` e variantes:
     * `Put { key: u64, value: String }`
     * `Get { key: u64 }`
     * `Delete { key: u64 }`
     * `Scan { start: u64, end: u64 }`
   * Essa modelagem unifica os campos comuns e específicos diretamente na raiz do objeto JSON de forma idiomática e sem redundâncias.

2. **Modelagem de Respostas (`Response`, `ResponseStatus`, `ScanRecord`):**
   * Struct `Response` com `id: u64`, `status: ResponseStatus`, e campos opcionais `value`, `records` e `message`.
   * `ResponseStatus` mapeia `Ok -> "ok"`, `NotFound -> "not_found"` e `Error -> "error"`.
   * Anotação `skip_serializing_if = "Option::is_none"` garante respostas compactas exatamente compatíveis com a especificação (ex.: `{"id": 1, "status": "ok"}`).

3. **Mecanismo de Despacho de Operações (`execute_request`):**
   * Encaminha `put` e `delete` para o `StorageEngine` (que força durabilidade com `fsync`).
   * Para `get`:
     * Se chave existir e for lida íntegra, converte os bytes persistidos para `String` UTF-8 e retorna `Response::ok_with_value(id, val)`.
     * Se a chave não existir ou estiver excluída por tombstone, retorna `Response::not_found(id)`.
     * Se houver falha de integridade (ex.: divergência de CRC32), retorna `Response::error(id, msg)`.
   * Para `scan`: retorna erro controlado na Etapa 1 indicando que a funcionalidade faz parte da Etapa 2.

4. **Processador em Streaming (`process_workload`):**
   * Itera sobre `R: BufRead` linha a linha.
   * Se uma linha apresentar JSON inválido, tenta extrair o campo `id` via parse genérico de fallback para reportar o erro preservando o identificador.
   * Serializa cada resposta diretamente em `W: Write` com quebra de linha e descarrega com `flush()` ao final do lote.
   * Coleta métricas consolidadas (`WorkloadStats`).

---

## 3. Alternativas Consideradas

* **Usar JSON genérico dinâmico (`serde_json::Value`) em todas as etapas:**
  - *Descartado:* Perde a segurança estrita de tipos em tempo de compilação do Rust e impõe alocações desnecessárias no heap a cada nó interpretado.
* **Carregar todas as requisições em um `Vec<Request>` antes de executar:**
  - *Descartado:* Viola frontalmente as regras contra carregar datasets na RAM e estoura a memória nos testes com grandes lotes de operações.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Conformidade total com o formato exigido na especificação oficial do trabalho.
  - Garantia de preservação estrita do campo `id` em todos os caminhos de execução.
  - Baixo consumo de memória através de pipeline linha a linha via streams bufferizados.
* **Compromissos:**
  - Parsing de JSON impõe um pequeno overhead de CPU em comparação com protocolos puramente binários, amortecido pelo desempenho da crate `serde_json`.
