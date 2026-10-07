# Architectural Decision Records (ADRs)

Este diretório armazena os registros de decisões arquiteturais (**Architecture Decision Records — ADRs**) do projeto **Adaptive Storage Engine**.

## 📌 Diretriz de Uso
Qualquer adição, alteração substancial ou escolha de design no projeto deve ser documentada de forma **atômica** em uma nota separada no formato `.md` neste diretório, descrevendo o contexto e justificando tecnicamente a escolha.

## 📋 Padrão de Nomenclatura
Os arquivos devem seguir o padrão sequencial:
`NNN-nome-da-decisao.md` (exemplo: `001-formato-registro-wal.md`, `002-modulo-erros.md`).

## 📐 Estrutura Recomendada para um ADR
Cada ADR deve conter:
1. **Título e Metadados:** Número, título claro, data e status (`Proposto`, `Aceito`, `Obsoleto`).
2. **Contexto e Problema:** Qual o desafio ou necessidade técnica sendo tratada.
3. **Decisão:** O que foi escolhido para resolver o problema.
4. **Justificativa e Alternativas Consideradas:** Por que essa abordagem foi escolhida e quais alternativas foram descartadas.
5. **Consequências:** Impactos positivos, limitações e compromissos (*trade-offs*) assumidos.

---

## 📚 Índice de Decisões Registradas

| ADR | Título | Status | Data |
| :--- | :--- | :---: | :---: |
| [001](001-modulo-de-erros-e-tipos-basicos.md) | Módulo de Erros e Tipos Básicos (`src/error.rs`) | Aceito | 2026-10-07 |
| [002](002-formato-binario-e-serializacao-wal.md) | Formato Binário e Serialização de Registros do WAL | Aceito | 2026-10-07 |
| [003](003-gravacao-append-only-e-durabilidade-wal.md) | Gravação Append-Only e Durabilidade do WAL (`src/wal/writer.rs`) | Aceito | 2026-10-07 |
| [004](004-varredura-sequencial-e-recuperacao-crash.md) | Varredura Sequencial, Leitura Pontual e Recuperação Pós-Crash do WAL | Aceito | 2026-10-07 |
| [005](005-nucleo-do-engine-e-indice-em-memoria.md) | Núcleo do Engine e Índice em Memória (Bitcask-Style) | Aceito | 2026-10-07 |
| [006](006-protocolo-json-lines.md) | Protocolo JSON Lines (JSONL) e Despacho de Operações | Aceito | 2026-10-07 |
| [007](007-interface-de-linha-de-comando.md) | Interface de Linha de Comando (CLI) (`src/cli.rs`) | Aceito | 2026-10-07 |


