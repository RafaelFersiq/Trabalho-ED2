# Etapa Atual: Entrega 1 — Persistent Storage Engine (Peso: 25%)

Este documento é a referência operacional direta para a execução da **Etapa 1** do projeto. Ele detalha os objetivos, decisões técnicas, arquitetura a ser implementada nesta fase e a lista sequencial de tarefas com checkboxes.

---

## 1. Detalhamento da Etapa 1

### 1.1 Objetivo e Escopo Funcional
A Etapa 1 tem como meta construir a fundação de persistência e recuperação do storage engine em disco:
* **Operações Obrigatórias:**
  * `PUT(key, value)`: Insere ou sobrescreve o valor associado a uma chave inteira `u64`. Valores possuem tamanho variável em bytes UTF-8.
  * `GET(key)`: Recupera o valor mais recente persistido para a chave. Retorna `status: "ok"` com o campo `value`, ou `status: "not_found"` caso a chave não exista ou tenha sido removida.
  * `DELETE(key)`: Remove a chave logicamente gravando um marcador de exclusão (*tombstone*). Retorna `status: "ok"`.
* **Persistência e Tolerância a Crash:**
  * Todos os dados confirmados devem sobreviver ao encerramento normal e a reinicializações.
  * O sistema deve tolerar interrupções abruptas (*crashes* de energia ou `SIGKILL`) sem perder registros previamente confirmados com `status: "ok"`.
  * Na inicialização, qualquer registro gravado parcialmente no final do arquivo de log deve ser detectado via validação de checksum e truncado com segurança.
* **Interface de Linha de Comando (CLI):**
  * `init --data-dir <DIR>`: Inicializa o diretório e seus arquivos de metadados.
  * `run --data-dir <DIR> --input <IN.jsonl> --output <OUT.jsonl>`: Executa operações em lote via JSONL com buffers.
  * `verify --data-dir <DIR>`: Realiza a checagem completa de integridade dos registros persistidos.
  * `describe`: Imprime dados da equipe e funcionalidades implementadas.

---

### 1.2 Decisões Técnicas e Arquitetura da Etapa 1

1. **Formato Binário de Registro (Append-Only Log / WAL):**
   Cada entrada gravada sequencialmente no arquivo de dados possui o layout de bytes:
   ```
   +---------------+------------+--------------+---------------+-----------------+
   | CRC32 (4 B)   | Flags (1B) | Key (8 B)    | ValLen (4 B)  | Value (N Bytes) |
   +---------------+------------+--------------+---------------+-----------------+
   ```
   * `CRC32` (`u32` little-endian): Checksum calculado com `crc32fast` sobre `Flags + Key + ValLen + Value`.
   * `Flags` (`u8`): `0x01` para registro ativo (`PUT`), `0x02` para marcador de remoção (*tombstone* / `DELETE`).
   * `Key` (`u64` little-endian): Chave inteira sem sinal de 64 bits.
   * `ValLen` (`u32` little-endian): Tamanho do payload de valor em bytes (0 para tombstones).
   * `Value`: Sequência de bytes do valor.

2. **Índice em Memória (Estilo Bitcask):**
   * Estrutura: `HashMap<u64, RecordLocation>`, onde `RecordLocation` guarda o deslocamento em bytes (`offset`) e tamanho do registro no arquivo de log.
   * `PUT` / `DELETE`: Escreve no final do arquivo de log em disco, força a sincronização com disco (`fsync`), e atualiza o mapa em memória (ou remove a chave se for tombstone).
   * `GET`: Busca em $O(1)$ a posição no arquivo via mapa, posiciona o cursor (`seek`), lê os bytes exatos, valida o CRC32 e extrai o valor.

3. **Recuperação e Tolerância a Falhas:**
   * Na abertura do engine, faz-se a varredura linear do arquivo de log desde o início.
   * Cada registro tem seu CRC32 validado.
   * Ao encontrar um registro incompleto ou corrompido no final do arquivo (típico de queda durante a escrita), o engine trunca o arquivo no último byte íntegro antes de aceitar novas requisições.

4. **Comando `verify`:**
   * Varre todos os registros do log em disco do início ao fim sem carregar todos os dados na RAM.
   * Valida o CRC32 de cada registro e verifica que os offsets e tamanhos são consistentes.
   * Emite relatório de integridade com código de saída limpo ou de erro explicativo em caso de corrupção.

---

## 2. Checklist Sequencial de Implementação da Etapa 1

- [x] **1. Módulo de Erros e Tipos Básicos (`src/error.rs`)**
  - [x] Definir `EngineError` (I/O, corrupção de CRC32, parsing de JSONL, formato de registro inválido).
  - [x] Implementar conversões de erro (`From<io::Error>`, `From<serde_json::Error>`).

- [x] **2. Formato Binário e Serialização de Registros (`src/wal/record.rs`)**
  - [x] Definir constantes de flags (`FLAG_PUT = 0x01`, `FLAG_DELETE = 0x02`).
  - [x] Implementar struct `LogRecord` (`key`, `value`, `is_tombstone`).
  - [x] Implementar serialização binária com cálculo de CRC32 (`crc32fast`).
  - [x] Implementar deserialização binária e validação estrita de CRC32.

- [ ] **3. Gravação Append-Only e Durabilidade (`src/wal/mod.rs` / `writer.rs`)**
  - [ ] Implementar estrutura do arquivo de log (`data.wal` ou similar sob `data-dir`).
  - [ ] Adicionar suporte a escrita bufferizada com flush explícito e `fsync` por operação de commit.
  - [ ] Retornar o offset e tamanho gravados para atualização do índice.

- [ ] **4. Varredura Sequencial e Recuperação de Crash (`src/wal/reader.rs`)**
  - [ ] Implementar iterador/leitor de registros do log desde o início do arquivo.
  - [ ] Tratar fim de arquivo inesperado ou corrupção no último registro com truncamento seguro (`file.set_len(valid_offset)`).
  - [ ] Implementar suporte à leitura pontual por offset (`seek` + `read_exact`) para consultas `GET`.

- [ ] **5. Núcleo do Engine e Índice em Memória (`src/engine.rs`)**
  - [ ] Implementar struct `StorageEngine` com mapa em memória (`HashMap<u64, RecordLocation>`).
  - [ ] Implementar procedimento de recuperação na abertura (`StorageEngine::open(data_dir)`).
  - [ ] Implementar `put(key, value)`.
  - [ ] Implementar `get(key)` retornando `Option<Vec<u8>>`.
  - [ ] Implementar `delete(key)` gravando tombstone e atualizando o índice.

- [ ] **6. Protocolo JSON Lines (`src/protocol.rs`)**
  - [ ] Modelar mensagens de entrada (`Request`: `id`, `op`, `key`, `value`, `start`, `end`).
  - [ ] Modelar mensagens de saída (`Response`: `id`, `status`, `value`, `records`).
  - [ ] Garantir preservação estrita do campo `id` em toda resposta gerada.

- [ ] **7. Interface de Linha de Comando (CLI) (`src/cli.rs`)**
  - [ ] Configurar comandos com `clap`: `init`, `run`, `verify`, `describe`.
  - [ ] Implementar `init`: criar diretório se não existir e metadados iniciais.
  - [ ] Implementar `run`: streaming linha a linha com `BufReader` e `BufWriter`.
  - [ ] Implementar `verify`: validação integral dos dados persistidos no diretório.
  - [ ] Implementar `describe`: impressão estruturada das informações da equipe e recursos suportados.

- [ ] **8. Ponto de Entrada Principal (`src/main.rs`)**
  - [ ] Integrar CLI, despacho de comandos e tratamento global de erros.

- [ ] **9. Suíte de Testes Automatizados da Etapa 1 (`tests/`)**
  - [ ] Teste unitário de integridade de registros binários e CRC32.
  - [ ] Teste de persistência: reinicialização do processo preservando dados de `PUT` e exclusões de `DELETE`.
  - [ ] Teste de crash recovery: simular escrita parcial no fim do log e validar truncamento e integridade dos registros anteriores.
  - [ ] Teste de ponta a ponta com JSONL via comando `run`.
