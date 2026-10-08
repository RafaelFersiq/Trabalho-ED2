# ADR-009: Suíte de Testes Automatizados da Etapa 1 (`tests/`)

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

A especificação da Etapa 1 exige comprovação rigorosa e automatizada de todas as propriedades fundamentais do storage engine:
1. **Integridade Física Binária:** Conformidade com o layout de registro em disco Little-Endian e validação exata de checksums CRC32 via `crc32fast`, com detecção determinística de qualquer corrupção em cabeçalhos, payloads ou flags.
2. **Durabilidade e Persistência:** Sobrevivência completa de dados inseridos (`PUT`), atualizados (sobrescritas) e excluídos logicamente (`DELETE` / tombstones) através de encerramentos e reinicializações de processo (`StorageEngine::open`).
3. **Tolerância a Falhas Abruptas (*Crash Recovery*):** Capacidade de detectar corrupção e gravações incompletas no final do log (típicas de `SIGKILL` ou interrupção de energia), executando truncamento limpo no último registro válido sem mascarar corrupções no meio do arquivo.
4. **Conformidade de Ponta a Ponta com o Protocolo JSONL:** Processamento em lote via comando CLI `run` com buffers em streaming (`BufReader`/`BufWriter`), preservação estrita do identificador `id` e integração com os subcomandos `init`, `verify` e `describe`.

Para garantir que a biblioteca e o binário possam ser testados como caixas-pretas de integração, os testes devem residir no diretório de integração padrão do Cargo (`tests/`), utilizando diretórios e arquivos temporários isolados (`tempfile`).

---

## 2. Decisão

Desenvolveu-se uma suíte de testes de integração modular dividida em quatro arquivos especializados em `tests/`:

1. **`tests/wal_binary_crc_test.rs` (Integridade de Registros e CRC32):**
   - Testa roundtrips completos de codificação e decodificação para operações `PUT` e `DELETE`.
   - Cobre payloads vazios (0 bytes), payloads extensos (64 KB) e sequências binárias arbitrárias.
   - Valida determinismo do algoritmo de CRC32.
   - Força corrupção de bits isolados em cada campo do registro (flags, key, val_len, value e no próprio campo CRC) assegurando que `LogRecord::decode` rejeite os dados corrompidos com `EngineError::CrcMismatch` ou `EngineError::InvalidRecord`.
   - Valida detecção de `UnexpectedEof` em cabeçalhos e payloads truncados.
   - Valida a proibição de tombstones com `val_len > 0`.

2. **`tests/persistence_reopen_test.rs` (Persistência e Reabertura do Engine):**
   - Valida a retenção de dados após o drop e subsequente `StorageEngine::open` no mesmo diretório.
   - Comprova que sobrescritas preservam a versão mais recente e ignoram versões anteriores no índice em RAM.
   - Garante que tombstones de `DELETE` eliminem chaves do índice, impedindo vazamento de registros removidos (`status: not_found`).
   - Testa múltiplos ciclos sucessivos de abertura, escrita, exclusão e verificação acumulando histórico no WAL.
   - Avalia cenários com centenas de chaves e valida a consistência de metadados e arquivos via `handle_verify`.

3. **`tests/crash_recovery_test.rs` (Tolerância a Falhas e Recuperação Pós-Crash):**
   - Simula gravação parcial no cabeçalho ao final do arquivo WAL (ex.: 7 bytes gravados dos 17 necessários), confirmando que a recuperação trunca os bytes residuais e reconstrói o índice com todos os registros anteriores intactos.
   - Simula gravação parcial no payload do valor ao final do WAL, confirmando truncamento e restauração íntegra.
   - Simula registro final completo com CRC corrompido, validando a identificação da falha de crash e reparo com segurança.
   - Assegura que o storage engine permite novas gravações normais logo após o truncamento.
   - **Regra de Ouro da Integridade:** Comprova que corrupções físicas no meio do log de dados (onde registros válidos subsequentes existem) **não** são truncadas silenciosamente, retornando `Err(EngineError::CrcMismatch)` e bloqueando o engine para evitar perda não detectada de dados.

4. **`tests/jsonl_workload_e2e_test.rs` (Ponta a Ponta com Workloads JSONL e CLI):**
   - Executa workloads completos via `handle_run` com arquivos reais de entrada e saída.
   - Testa a preservação estrita do campo `id` em todas as respostas geradas, incluindo IDs arbitrários (`0`, `u64::MAX`, não sequenciais).
   - Valida o status `not_found` em consultas de chaves inexistentes ou excluídas.
   - Valida a mensagem informativa e status `error` para operações `SCAN` (cujo suporte funcional pertence à Etapa 2).
   - Testa a ignorância segura de linhas em branco e espaços vazios no fluxo JSONL.
   - Exercita o fluxo completo de subcomandos via `execute_cli`: `init` $\rightarrow$ `run` $\rightarrow$ `verify` $\rightarrow$ `describe`.

---

## 3. Justificativa e Alternativas Consideradas

* **Testes unitários internos (`#[cfg(test)]`) vs. Testes de integração (`tests/`):**
  - Embora os módulos internos em `src/` já possuam testes unitários pontuais (47 em `lib.rs` e 6 em `main.rs`), testes em `tests/` são compilados como crates independentes que consomem a API pública exportada por `storage_engine`, exercitando a interação real entre subsistemas (I/O em disco, arquivos reais, sistema de arquivos do SO e CLI).
* **Simulação de Crash via injeção em arquivo vs. Interrupção real por `SIGKILL`:**
  - A injeção física de bytes parciais e corrompidos diretamente no arquivo `data.wal` no sistema de arquivos é determinística, reprodutível e cobre exatamente as condições de disco deixadas por uma interrupção súbita durante o `write`/`flush`, permitindo asserções exatas de bytes truncados e offsets esperados.

---

## 4. Consequências e Resultados

* **Cobertura Completa:** O projeto atinge 88 testes automatizados (53 unitários e 35 de integração), todos executando com 100% de aprovação em frações de segundo.
* **Segurança de Regressão:** Qualquer alteração no layout binário, serialização JSONL ou fluxo de recuperação pós-crash é imediatamente detectada pela suíte de testes.
* **Prontidão para Submissão:** A Etapa 1 atinge todos os critérios de corretude e resiliência exigidos pelo ED2Bench e pelas regras formais da disciplina.
