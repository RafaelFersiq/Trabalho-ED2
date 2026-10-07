# ADR-007: Interface de Linha de Comando (CLI)

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

A interface oficial de interação externa com o *Storage Engine* nas avaliações automatizadas (ED2Bench) e execução local/Docker é uma **Interface de Linha de Comando (CLI)** padronizada com quatro comandos mandatórios:
1. `init --data-dir <DIR>`: Inicializa o diretório de dados com os arquivos e metadados requeridos.
2. `run --data-dir <DIR> --input <IN.jsonl> --output <OUT.jsonl>`: Executa um lote de operações do workload em JSON Lines (JSONL).
3. `verify --data-dir <DIR>`: Audita a integridade física de todos os registros persistidos no diretório.
4. `describe`: Exibe metadados, identificação da equipe e recursos arquiteturais implementados.

Os principais requisitos de engenharia incluem:
- **Conformidade Estrita com Nomes e Argumentos:** Os comandos e parâmetros devem aceitar exatamente as opções longas especificadas (`--data-dir`, `--input`, `--output`) sem desvios.
- **Eficiência de I/O e Streaming no `run`:** O processamento do workload não pode carregar arquivos inteiros na memória RAM; deve utilizar buffers (`BufReader` e `BufWriter`) em streaming linha a linha.
- **Rigor na Verificação (`verify`):** Não mascarar erros físicos de CRC32; se houver corrupção física no WAL, o comando deve falhar com código de erro explícito.
- **Persistência de Metadados (`init`):** O comando deve ser idempotente, criando o diretório se necessário, o arquivo `metadata.json` estruturado e garantindo a prontidão do Write-Ahead Log (`data.wal`).
- **Autodescrição (`describe`):** Retornar informações completas da equipe, versão, etapa e funcionalidades suportadas em formato estruturado (JSON).

---

## 2. Decisão

Implementou-se o módulo `src/cli.rs` integrando a crate `clap` (com macro `derive`) ao núcleo do engine:

1. **Modelagem dos Argumentos (`Cli` e `Commands`):**
   - Utilizou-se `#[derive(Parser)]` para a struct raiz `Cli` e `#[derive(Subcommand)]` para o enum `Commands`.
   - Argumentos possuem bindings explícitos `short` e `long` (`--data-dir` / `-d`, `--input` / `-i`, `--output` / `-o`), com validação e tipagem forte via `std::path::PathBuf`.
   - Metadados do executável (`name`, `version`, `author`, `about`) são herdados automaticamente das configurações do `Cargo.toml`.

2. **Inicialização de Diretório (`handle_init`):**
   - Garante a criação de diretórios aninhados via `fs::create_dir_all`.
   - Gera o arquivo `metadata.json` contendo `engine`, `version`, `format_version` e `stage`, validando a integridade se o arquivo já existir (idempotência segura).
   - Aciona `StorageEngine::open(data_dir)` para inicializar o arquivo de WAL (`data.wal`) limpo ou executar reparo preliminar caso já existisse.

3. **Execução de Workload (`handle_run`):**
   - Valida previamente a existência do arquivo de entrada e garante os diretórios pais do arquivo de saída.
   - Conecta `BufReader::new(File::open(input))` e `BufWriter::new(File::create(output))` à função `process_workload` do módulo `protocol`.
   - Força o esvaziamento completo dos buffers de saída (`writer.flush()`) e a sincronização do engine em disco (`engine.sync()`), assegurando durabilidade física total antes da conclusão.

4. **Auditoria de Integridade (`handle_verify`):**
   - Valida a presença e integridade do `metadata.json`.
   - Audita o arquivo WAL do início ao fim usando `WalReader::verify_file`, recalculando e validando o CRC32 de cada registro.
   - Retorna a contagem exata de registros e bytes auditados, propagando imediatamente erros de checksum (`EngineError::CrcMismatch`) caso haja corrupção física.

5. **Exibição Estruturada de Metadados (`handle_describe`):**
   - Retorna a struct `DescribeReport` contendo equipe, versão, etapa, operações e diferenciais técnicos.
   - Serializa a saída em JSON formatado (`serde_json::to_string_pretty`), facilitando inspeção legível e consumo por oráculos automatizados de avaliação.

---

## 3. Alternativas Consideradas

* **Parsing manual de `std::env::args()`:**
  - *Descartado:* Propenso a falhas de robustez em flags opcionais, mensagens de ajuda manuais e falta de validação idiomática fornecida pelo `clap`.
* **Imprimir texto puro não-estruturado no `describe`:**
  - *Descartado:* Formato JSON é o padrão preferencial em benchmarks e testes automatizados, permitindo que oráculos façam parse de metadados programaticamente.
* **Manter a lógica dos comandos diretamente na função `main`:**
  - *Descartado:* Separar o tratamento dos comandos em funções modulares e testáveis (`handle_init`, `handle_run`, `handle_verify`, `handle_describe`) em `src/cli.rs` permite criar testes unitários e de integração sem invocar subprocessos.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Plena conformidade com a especificação formal de CLI exigida pelo projeto (`init`, `run`, `verify`, `describe`).
  - Total cobertura por testes unitários automatizados validando tanto a sintaxe do parser quanto o comportamento de ponta a ponta dos handlers.
  - Baixo uso de memória mantido com streaming bufferizado no comando `run`.
* **Compromissos:**
  - Dependência direta da crate `clap` no binário final (já previamente autorizada nas regras da disciplina).
