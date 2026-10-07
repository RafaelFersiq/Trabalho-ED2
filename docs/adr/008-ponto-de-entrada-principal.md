# ADR-008: Ponto de Entrada Principal e Tratamento Global de Erros (`src/main.rs`)

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

O projeto exige a entrega de um binário executável autônomo denominado `/engine` (gerado a partir de `src/main.rs`), operando em conformidade estrita com as especificações da disciplina ED2.

O ponto de entrada principal do binário precisa:
1. Integrar o analisador sintático de argumentos de linha de comando (`clap::Parser`).
2. Despachar a execução para o subsistema de comandos da CLI (`storage_engine::cli::execute_cli`).
3. Tratar de forma centralizada e padronizada qualquer falha operacional ou corrupção de integridade retornada pelo motor de armazenamento.
4. Produzir mensagens de diagnóstico claras e estruturadas na saída de erro padrão (`stderr`), expondo a cadeia de causas do erro (através de `std::error::Error::source`).
5. Finalizar o processo com códigos de saída semânticos e canônicos:
   - Código `0` para execuções bem-sucedidas ou consultas de autoajuda/versão (`--help`, `--version`).
   - Código `1` para falhas de execução interna, erros de I/O, falhas de CRC32 ou inconsistências lógicas.
   - Código `2` para argumentos inválidos ou malformados na linha de comando (gerenciado de forma nativa pelo `clap`).

---

## 2. Decisão

Adotou-se a seguinte estrutura arquitetural para `src/main.rs`:

1. **Separação entre Pipeline de Execução e Ponto de Entrada:**
   - Implementou-se a função auxiliar `run_app() -> Result<()>`, responsável por realizar o parsing de argumentos do ambiente e repassar a estrutura tipada `Cli` para `storage_engine::cli::execute_cli`.
   - Essa separação mantém a função `main()` estritamente focada no ciclo de vida do processo (captura de erro, formatação de diagnósticos e terminação via código de saída).

2. **Exibição Estruturada da Cadeia de Erros (`print_error`):**
   - Criou-se a função `print_error(err: &(dyn Error + 'static))` que formata a mensagem primária de erro e itera recursivamente sobre a cadeia de causas via `err.source()`, imprimindo cada nível com o prefixo `Causa:` no canal `stderr`.
   - Isso garante diagnósticos precisos em cenários onde erros de I/O ou desserialização JSON mascaram problemas subjacentes de permissão ou caminho de arquivo.

3. **Gerenciamento Canônico de Códigos de Saída (`process::exit`):**
   - Em caso de retorno `Err(err)` proveniente de `run_app()`, a aplicação emite a mensagem de erro formatada e finaliza imediatamente com código `1`.
   - Sucessos encerram o processo normalmente sem forçar chamadas adicionais, retornando `0` ao sistema operacional.

4. **Testabilidade:**
   - A inclusão de testes unitários diretamente no módulo `main.rs` valida o comportamento de formatação de erros (`print_error`), o tratamento de fontes encadeadas e o despacho de subcomandos válidos e inválidos em conjunto com diretórios temporários (`tempfile`).

---

## 3. Alternativas Consideradas

* **Utilizar `unwrap()` ou `expect()` diretamente na `main`:**
  - *Descartado:* Provocaria um *panic* com *stack trace* ruidoso na saída do usuário, violando as boas práticas de usabilidade de ferramentas CLI e dificultando a depuração em testes automatizados.
* **Retornar `Result<(), EngineError>` diretamente da função `main`:**
  - *Descartado:* A implementação padrão de `Termination` do Rust para `Result` imprime a representação de depuração (`{:?}`) em vez da mensagem formatada para usuário final (`{}`), omitindo detalhes limpos de formatação e navegação na cadeia de causas (`source()`).
* **Acoplar o parsing de argumentos e despacho diretamente dentro da `main` sem modularização:**
  - *Descartado:* Dificulta a cobertura por testes automatizados e gera código monolítico. A delegação para `storage_engine::cli::execute_cli` mantém alta coesão e baixo acoplamento.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Executável robusto e confiável, pronto para empacotamento em contêiner Docker sob `/engine`.
  - Mensagens de erro informativas no `stderr` sem ruído de pânico.
  - Plena conformidade com as regras de avaliação automatizada do ED2Bench.
* **Compromissos:**
  - O encerramento imediato via `std::process::exit(1)` não executa destructors de variáveis ainda ativas no escopo da `main`, motivo pelo qual todo o trabalho de buffers e sincronização de dados com o disco (`writer.flush()`, `engine.sync()`) é garantido previamente dentro de `handle_run` e `execute_cli`.
