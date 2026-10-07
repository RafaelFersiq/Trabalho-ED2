# ADR-001: Módulo Centralizado de Erros e Tipos Básicos

- **Status:** Aceito
- **Data:** 2026-10-07
- **Autor:** Equipe ED2

---

## 1. Contexto e Problema

O **Adaptive Storage Engine** deve operar diretamente sobre arquivos binários em disco, gerenciando durabilidade com `fsync`, integridade com CRC32, parsing de mensagens JSONL e recuperação de estado após quedas abruptas (*crashes*).

Para garantir os critérios inegociáveis de corretude e robustez:
1. O motor não pode realizar *panics* em tempo de execução ao encontrar erros operacionais previsíveis (disco cheio, corrupção de dados, entradas malformadas).
2. O sistema de recuperação (*crash recovery*) precisa diferenciar confiavelmente entre:
   - Um erro de I/O do sistema operacional;
   - Uma corrupção física ou término abrupto de registro no fim do log (que requer truncamento seguro);
   - Registros inválidos ou corrupções estruturais no meio do arquivo (que exigem parada com erro).
3. A integração com ferramentas externas (`std::io`, `serde_json`) deve ser idiomática, permitindo a propagação de erros via operador `?`.
4. Os tipos primitivos do domínio (`Key = u64`, `RecordLocation`) devem ser tipados e compartilhados de forma consistente por todos os módulos.

---

## 2. Decisão

1. **Enum `EngineError` centralizado em `src/error.rs`:**
   - `Io(std::io::Error)`: Erros de I/O subjacentes do sistema operacional.
   - `CrcMismatch { expected: u32, calculated: u32, offset: u64 }`: Falha na checagem de integridade de checksum.
   - `UnexpectedEof`: Leitura interrompida antes do fim esperado de um cabeçalho ou payload.
   - `InvalidRecord(String)`: Falhas lógicas de decodificação binária (ex.: flags desconhecidas, comprimentos inconsistentes).
   - `Json(serde_json::Error)`: Falhas na serialização ou deserialização do protocolo JSON Lines.
   - `Corruption(String)`: Inconsistências gerais de integridade lógica ou metadados de diretório.

2. **Implementação de Traits Padrão:**
   - `std::fmt::Display` e `std::error::Error` implementados manualmente, mantendo dependências mínimas e zero overhead.
   - `From<std::io::Error>` e `From<serde_json::Error>` para conversão automática via operador `?`.

3. **Tipos Básicos e Aliases de Domínio:**
   - `pub type Result<T> = std::result::Result<T, EngineError>;`
   - `pub type Key = u64;`
   - `pub struct RecordLocation { pub offset: u64, pub value_len: u32 }` (usado pelo índice Bitcask em memória).

4. **Exposição em Biblioteca (`src/lib.rs`):**
   - Criar `src/lib.rs` exportando o módulo `error` para permitir o reaproveitamento do código tanto pelo binário (`src/main.rs`) quanto pelos testes de integração em `tests/`.

---

## 3. Alternativas Consideradas

* **Uso de crates como `thiserror` ou `anyhow`:**
  - *Descartado:* O uso de traits manuais do Rust padrão mantém a base de código enxuta, sem dependências adicionais, e facilita a explicação e defesa técnica na disciplina.
* **Erros representados como `String` ou `Box<dyn Error>`:**
  - *Descartado:* Dificulta o casamento de padrões (*pattern matching*) necessário no algoritmo de recuperação pós-crash (onde é preciso identificar especificamente `CrcMismatch` ou `UnexpectedEof`).

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Tratamento determinístico e tipado de todas as falhas operacionais do motor.
  - Facilidade de inspeção em testes unitários e de integração.
  - Código idiomático em Rust com suporte ao operador `?`.
* **Compromissos:**
  - Manutenção manual de mensagens em `Display` ao introduzir novas variantes de erro.
