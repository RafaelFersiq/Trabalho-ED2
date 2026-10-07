# ADR-004: Varredura Sequencial, Leitura Pontual e Recuperação Pós-Crash do WAL

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

Para atender às garantias de integridade e às operações da Etapa 1 do storage engine, o subsistema de leitura do Write-Ahead Log (WAL) deve solucionar três necessidades centrais:
1. **Leitura Pontual em $O(1)$ para Consultas `GET`:** As operações de leitura usam o índice em memória para localizar o offset do registro no log em disco. Ao ler esse registro, é obrigatório validar o checksum CRC32 antes de retornar o valor ao cliente, impedindo a entrega de dados corrompidos.
2. **Varredura Sequencial com Baixo Consumo de RAM:** Na inicialização e na execução do comando `verify`, o WAL deve ser percorrido sequencialmente sem carregar todo o arquivo ou dataset na memória RAM, sob risco de falha por *Out Of Memory* (OOM).
3. **Tolerância a Crashes e Truncamento Seguro:** Durante uma falha súbita de energia ou encerramento abrupto do processo (`SIGKILL`), a última operação de escrita pode ter sido gravada apenas parcialmente (cabeçalho incompleto, payload truncado ou bytes corrompidos). O sistema deve ser capaz de distinguir com precisão uma falha de escrita no final do arquivo de uma corrupção física no meio do arquivo, truncando o log no último byte íntegro sem mascarar corrupções no histórico persistido.

---

## 2. Decisão

1. **Abstração `WalReader` (`src/wal/reader.rs`):**
   * Oferece métodos de leitura pontual [`read_record_at`](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/reader.rs#L58) e [`read_value_at`](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/reader.rs#L70), posicionando o cursor via `seek` no offset indicado e validando o CRC32 integralmente via `LogRecord::decode`.
   * Rejeita consultas de leitura a marcadores de exclusão (*tombstones*), mantendo a garantia de isolamento.

2. **Iterador Sequencial em Streaming (`WalIterator`):**
   * Encapsula um `BufReader<File>` para processar registros sucessivos sob demanda (`next_entry`).
   * Retorna structs [`WalEntry`](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/reader.rs#L12) contendo o registro e seu `RecordLocation`, permitindo que os bytes do valor sejam descartados da memória imediatamente após a atualização do índice em RAM.

3. **Mecanismo de Recuperação e Reparo Pós-Crash (`recover_and_repair`):**
   * Percorre o arquivo de log do byte 0 em diante, repassando cada entrada válida para um callback `on_entry`.
   * Quando uma falha de decodificação ocorre (`UnexpectedEof`, `CrcMismatch` ou `InvalidRecord`):
     * Executa uma sonda de leitura (`read probe`) para verificar se o cursor atingiu o final físico do arquivo (`EOF`).
     * **No Final do Arquivo (Crash):** Trunca o arquivo físico em disco para o último offset íntegro confirmado (`valid_offset`) via `set_len()` e força a sincronização com `sync_all()`, retornando um [`RecoveryReport`](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/reader.rs#L24). O escritor (`WalWriter`) pode reabrir o arquivo e continuar anexando a partir deste ponto.
     * **No Meio do Arquivo (Corrupção):** Caso ainda existam bytes após o registro com erro, a rotina não mascara a falha e retorna imediatamente o erro de integridade (`EngineError::CrcMismatch`).

4. **Auditoria Estrita de Integridade (`verify_file`):**
   * Varre o arquivo sem realizar qualquer alteração ou truncamento, retornando a contagem de registros válidos e bytes validados ou erro imediato caso qualquer anomalia seja detectada.

---

## 3. Alternativas Consideradas

* **Carregar todos os registros em `Vec<WalEntry>` na recuperação:**
  - *Descartado:* Viola a regra mandatória do trabalho contra carregar o dataset em memória, pois registros grandes de benchmarks fariam o consumo de RAM estourar os limites estipulados.
* **Truncar o arquivo silenciosamente em qualquer erro de CRC (inclusive no meio):**
  - *Descartado:* Viola a regra "Não mascarar erros de corrupção física". Corrupções no meio do histórico de transações devem ser acusadas pelo engine.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Tolerância comprovada contra crashes e gravações parciais no fim do log.
  - Segurança de dados: toda leitura pontual valida o checksum CRC32.
  - Eficiência de memória com streaming por registros.
* **Compromissos:**
  - A leitura pontual para `GET` incorre em syscall de `seek` + leitura do cabeçalho e payload, amortecida por buffers do SO ou futura introdução de MemTable/caches nas próximas etapas.
