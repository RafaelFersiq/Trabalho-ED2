# ADR-002: Layout Binário e Serialização de Registros do WAL

- **Status:** Aceito
- **Data:** 2026-10-07
- **Autor:** Equipe ED2

---

## 1. Contexto e Problema

O Write-Ahead Log (WAL) é o mecanismo central de durabilidade física da Etapa 1 do projeto. Toda operação confirmada (`PUT` ou `DELETE`) precisa ser serializada sequencialmente em disco de modo que:
1. Garanta integridade contra corrupção física ou falhas de hardware/setores.
2. Permita detecção precisa do ponto de truncamento em caso de interrupção abrupta (*crash recovery*).
3. Seja compacta e eficiente, minimizando o volume de I/O em disco.
4. Permita extração rápida do tamanho total do registro para manter offsets no índice em memória (*Bitcask style*).

---

## 2. Decisão

1. **Layout Físico do Registro:**
   O registro binário é serializado com cabeçalho fixo de 17 bytes em codificação *Little-Endian*:
   ```
   +---------------+------------+--------------+---------------+-----------------+
   | CRC32 (4 B)   | Flags (1B) | Key (8 B)    | ValLen (4 B)  | Value (N Bytes) |
   +---------------+------------+--------------+---------------+-----------------+
   ```
   * `CRC32` (`u32` little-endian, bytes 0..3): Checksum calculado com aceleração SIMD (`crc32fast::Hasher`) sobre o bloco `[Flags, Key, ValLen, Value]`.
   * `Flags` (`u8`, byte 4): `FLAG_PUT = 0x01` para registros ativos, `FLAG_DELETE = 0x02` para marcadores de exclusão (*tombstones*).
   * `Key` (`u64` little-endian, bytes 5..12): Chave inteira sem sinal de 64 bits.
   * `ValLen` (`u32` little-endian, bytes 13..16): Comprimento do vetor de valor em bytes (obrigatório 0 em `FLAG_DELETE`).
   * `Value` (bytes 17..17+N): Sequência de bytes brutos representando o valor.

2. **Detecção e Tratamento de Fim de Arquivo e Erros:**
   * **Clean EOF:** Retornado quando o leitor encontra 0 bytes no início da tentativa de leitura de um novo registro.
   * **Unexpected EOF:** Retornado quando a leitura é interrompida no meio do cabeçalho de 17 bytes ou antes de completar os `ValLen` bytes do payload (cenário característico de queda de energia durante gravação).
   * **CrcMismatch:** Disparado quando o cabeçalho e payload são lidos, mas o checksum calculado diverge do esperado.
   * **Proteção de Alocação (`MAX_VALUE_LEN`):** Teto defensivo (ex.: 64 MB) para evitar alocações abusivas de memória caso bytes corrompidos no disco indiquem um `ValLen` arbitrariamente grande antes da checagem de CRC.

3. **Mapeamento de Tipos:**
   Estrutura `LogRecord` em `src/wal/record.rs` expondo métodos construtores (`put`, `delete`), serialização (`encode`, `encode_to_vec`), deserialização (`decode`) e cálculo de tamanho total (`encoded_size`).

---

## 3. Alternativas Consideradas

* **JSON / Texto puro no WAL:**
  - *Descartado:* Formatos textuais impõem overhead massivo de parsing, geram arquivos significativamente maiores e impedem posicionamento determinístico por offset em disco via `seek`.
* **Checksum no final do registro:**
  - *Descartado:* Manter o CRC no início do cabeçalho permite que utilitários de inspeção e scanners leiam a assinatura imediatamente e comparem com o fluxo sequencial que segue.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Robustez comprovada contra escritas parciais e corrupção física.
  - Baixa sobrecarga (apenas 17 bytes de metadados por entrada).
  - Cálculo de CRC de alta performance acelerado por hardware via `crc32fast`.
* **Compromissos:**
  - Campos inteiros devem ser tratados estritamente como *little-endian* (`to_le_bytes` / `from_le_bytes` ou `byteorder`).
