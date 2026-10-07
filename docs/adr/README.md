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
