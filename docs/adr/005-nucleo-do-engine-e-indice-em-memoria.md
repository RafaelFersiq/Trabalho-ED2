# ADR-005: Núcleo do Engine e Índice em Memória (Bitcask-Style)

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

Com a conclusão do formato binário de registros (`LogRecord`), do escritor com suporte a `fsync` (`WalWriter`) e do leitor com recuperação pós-crash (`WalReader`), faz-se necessária a unificação desses componentes em uma interface de alto nível: o `StorageEngine`.

Os desafios e restrições mandatórias que guiam essa decisão incluem:
1. **Controle Rigoroso de Memória RAM:** O conjunto de dados manipulado pelos benchmarks é substancialmente maior do que o teto de RAM alocado para o container. Portanto, os valores de registros (com comprimentos arbitrários) **não podem** ser retidos na memória principal.
2. **Garantia de Durabilidade Imediata:** Nenhuma escrita ou exclusão pode ser considerada confirmada antes de ser fisicamente persistida no disco via `fsync`.
3. **Isolamento de Exclusões (*Tombstones*):** Registros excluídos não devem ser expostos a operações subsequentes de leitura, mas seus marcadores devem ser persistidos sequencialmente no log para sobreviver a reinicializações.
4. **Recuperação Determinística e Autocorreção na Abertura:** A abertura do engine deve reexecutar o log em streaming, reconstruir o estado ativo em memória e truncar registros parciais causados por falhas abruptas no fim do arquivo.

---

## 2. Decisão

1. **Abstração `StorageEngine` (`src/engine.rs`):**
   * Encapsula o caminho do diretório de dados (`data_dir`), o índice em memória (`HashMap<Key, RecordLocation>`), o escritor (`WalWriter`), o leitor pontual (`WalReader`) e o relatório de recuperação pós-crash (`RecoveryReport`).
   * Mantém o índice estritamente restrito a pares `Key -> RecordLocation`. O `RecordLocation` contém apenas `offset: u64` e `value_len: u32` (12 bytes), garantindo pegada de memória de ordem $O(N)$ em relação à quantidade de chaves, e $O(0)$ em relação ao tamanho dos valores.

2. **Fluxo de Inicialização e Recuperação (`StorageEngine::open`):**
   * Cria o diretório de dados caso inexistente.
   * Invoca `WalReader::recover_in_dir` processando as entradas sequencialmente por closure:
     * Registros normais (`PUT`): inserem ou atualizam a chave no `HashMap`.
     * Marcadores de exclusão (`DELETE`): removem a chave correspondente do `HashMap`.
     * O payload de valor (`Vec<u8>`) é descartado da RAM imediatamente ao término de cada iteração.
   * Em caso de falha de gravação no final do arquivo WAL por crash (`UnexpectedEof` ou CRC truncado), o leitor trunca o arquivo no último offset válido antes de prosseguir.
   * Abre o `WalWriter` posicionado no fim do log íntegro e o `WalReader` para leituras pontuais.

3. **Operações Primitivas:**
   * **`put(key, value)`:** Codifica o registro, invoca `WalWriter::write_and_sync` (garantindo persistência em disco via `fsync`) e atualiza o mapa em memória com o `RecordLocation` retornado.
   * **`get(key)`:** Busca a chave no `HashMap`. Se não existir, retorna `Ok(None)` imediatamente sem I/O. Se encontrada, utiliza `WalReader::read_value_at` para executar `seek` até o offset físico em disco, ler os bytes exatos, validar o checksum CRC32 e retornar o valor.
   * **`delete(key)`:** Grava sequencialmente um registro tombstone no WAL com `fsync` e remove a chave do mapa em memória. Retorna indicador booleano de existência prévia.

4. **Operações de Suporte e Diagnóstico:**
   * `verify()`: Executa validação física de todos os registros no arquivo WAL através de `WalReader::verify_file`.
   * `len()`, `is_empty()`, `contains_key()`, `recovery_report()`: Métodos de consulta ao estado interno e diagnóstico pós-recuperação.

---

## 3. Alternativas Consideradas

* **Manter Valores Pequenos em RAM (Cache no Índice):**
  - *Descartado:* Na Etapa 1, a simplicidade e o isolamento rígido contra OOM são prioritários. Caching estruturado em blocos (LRU/2Q) está planejado para a Etapa 3.
* **Confirmar Gravação em Memória e Fazer `fsync` Assíncrono:**
  - *Descartado:* Viola a regra formal do trabalho: *"Qualquer operação confirmada com status: 'ok' deve ser efetivamente gravada e sincronizada fisicamente no disco antes do envio da resposta"*.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Integridade ponta a ponta: todas as mutações sobrevivem a crashes e reinicializações.
  - Conformidade estrita com o limite de memória RAM através da separação entre índices e valores (modelo Bitcask).
  - Autocorreção transparente na inicialização sem perda de dados previamente confirmados.
* **Compromissos:**
  - Cada operação `get` de chave existente requer uma leitura em disco com `seek`. O impacto em latência é mitigado pelos buffers de página do sistema operacional e será aprimorado nas etapas seguintes com MemTable e Block Cache.
