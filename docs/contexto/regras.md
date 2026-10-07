# Regras do Trabalho: Delimitação Oficial

Este documento consolida as regras formais e operacionais do projeto **Adaptive Storage Engine (ED2)** com base em `docs/contexto/DOCUMENTACAO_PROJETO.md`. 
O objetivo é fornecer uma separação nítida entre o que é **obrigatório**, o que é **permitido** e o que é **estritamente proibido**.

---

## 1. O que DEVE ser feito (Obrigações e Requisitos Mandatórios)

### 1.1 Corretude e Integridade
* **Corretude é Pré-Requisito de Desempenho:** A conformidade dos dados e das respostas é prioritária. Respostas incorretas anulam a pontuação de desempenho no respectivo workload.
* **Preservação Rígida do Identificador (`id`):** Toda resposta gerada no protocolo JSONL deve conter o mesmo `id` da requisição recebida correspondente.
* **Persistência Real em Disco:** Qualquer operação confirmada com `status: "ok"` deve ser efetivamente gravada e sincronizada fisicamente no disco (`fsync` ou equivalente) antes do envio da resposta.
* **Validação de Integridade por Checksum (CRC32):** Todos os registros binários e blocos gravados em disco devem possuir checksums CRC32 validados em tempo de leitura e recuperação.
* **Recuperação Pós-Crash:** O sistema deve resistir a interrupções abruptas (*crashes* de energia ou encerramento forçado do processo). No religamento, registros parciais corrompidos no final do arquivo de log devem ser detectados pelo CRC32 e truncados de forma limpa.

### 1.2 Regras do Modelo de Dados e Protocolo
* **Tipos de Chave e Valor:**
  * Chaves devem ser inteiros sem sinal de 64 bits (`uint64` / `u64`).
  * Valores devem suportar tamanhos arbitrários e variáveis (sequência de bytes UTF-8 no JSON).
* **Semântica de Operações:**
  * `PUT`: Inserir nova chave ou sobrescrever integralmente o valor anterior.
  * `GET`: Retornar o valor mais recente ou `status: "not_found"` caso a chave não exista ou esteja deletada.
  * `DELETE`: Marcar a chave como excluída via *tombstone*.
  * `SCAN` (Etapas 2 e 3): Retornar registros no intervalo inclusivo `[start, end]` em **ordem estritamente crescente de chave**, sem duplicatas e sem chaves deletadas.
* **Interface CLI (`/engine`):**
  * Implementar obrigatoriamente os comandos: `init`, `run`, `verify` e `describe`.

### 1.3 Entregas e Critérios da Disciplina
* **Composição de Cada Entrega:** Cada uma das três etapas deve incluir:
  1. Código-fonte completo da solução;
  2. Artefato executável correspondente (`/engine` ou Docker);
  3. Documentação técnica do grupo;
  4. Relatório e resultados da análise experimental.
* **Registro de Ferramentas de IA:** O uso de IA deve ser acompanhado para a redação da seção obrigatória **AI-Assisted Engineering** no relatório final.
* **Capacidade de Defesa:** A equipe deve compreender integralmente todo o código e ser capaz de explicar e defender qualquer decisão arquitetural.

---

## 2. O que PODE ser feito (Permissões e Boas Práticas Autorizadas)

### 2.1 Uso de Bibliotecas e Ferramentas
* **Bibliotecas Auxiliares de Propósito Geral:** É expressamente permitido o uso de crates como:
  * `clap`: Para parsing e validação de argumentos da CLI.
  * `serde` e `serde_json`: Para serialização e parsing do protocolo JSONL.
  * `crc32fast`: Para cálculo de checksums de integridade com aceleração por hardware (SIMD).
  * `byteorder`: Para garantir leitura/escrita em formato binário Little-Endian.
  * `tempfile`: Para isolamento de testes e diretórios temporários.
* **Ferramentas de IA Generativa:** Uso amplamente livre para programar, debugar, estudar a teoria, gerar testes, documentar e analisar experimentos.
* **Otimizações de I/O em Memória Secundária:** Uso de buffers como `BufReader` e `BufWriter` para maximizar o throughput e reduzir o número de syscalls ao sistema operacional.

### 2.2 Arquitetura e Estruturas Internas
* **Liberdade de Projeto:** A equipe tem liberdade para definir os detalhes de sua arquitetura (ex.: LSM-Tree com Bitcask na Etapa 1, MemTable com SSTables na Etapa 2 e mecanismos adaptativos na Etapa 3), desde que devidamente justificados.
* **Índices Parciais em RAM:** É permitido manter índices em memória (como o mapeamento de offsets na Etapa 1 ou a MemTable com limite de tamanho configurável na Etapa 2), desde que respeitem os limites de memória RAM.
* **Tolerância a Falhas Isoladas:** Uma falha em um workload de teste específico anula a pontuação daquele teste, mas **não implica nota zero automática** em toda a entrega.

---

## 3. O que NÃO DEVE ser feito (Proibições Estritas e Falhas Críticas)

### 3.1 Proibições Formais
* ❌ **NÃO usar DBMSs Prontos:** É expressamente proibido delegar o armazenamento a bancos ou bibliotecas prontas (ex.: SQLite, RocksDB, LevelDB, Sled, DuckDB, LMDB, BerkeleyDB).
* ❌ **NÃO delegar paginação, índices ou persistência a bibliotecas externas:** A lógica de persistência, organização física dos blocos, índices e recuperação deve ser desenvolvida pela equipe.
* ❌ **NÃO utilizar código gerado por IA sem compreendê-lo:** A equipe não deve usar trechos de código que não seja capaz de explicar em detalhes ou defender tecnicamente.

### 3.2 Erros Críticos de Engenharia e Violações de Desempenho
* ❌ **NÃO carregar todo o dataset na memória RAM:** O volume de dados dos benchmarks excede a RAM disponível. Manter todos os registros em memória causará estouro (*OOM - Out of Memory*) e desclassificação.
* ❌ **NÃO confirmar escritas sem durabilidade física:** Nunca responder `status: "ok"` antes de gravar no arquivo e executar `flush` / `fsync`. Isso causa perda de dados confirmados em testes de crash.
* ❌ **NÃO omitir ou alterar os campos `id`:** O protocolo JSONL exige retorno estrito do mesmo identificador para cada linha de operação recebida.
* ❌ **NÃO deixar vazar registros deletados (*tombstones*):** Chaves removidas não podem ser retornadas em consultas `GET` nem no array de registros do `SCAN`.
* ❌ **NÃO retornar chaves fora de ordem ou duplicadas no `SCAN`:** O retorno deve ser estritamente monotônico crescente por chave.
* ❌ **NÃO mascarar erros de corrupção física:** Se um bloco no meio do arquivo de dados estiver com CRC32 inválido, o comando `verify` e as operações afetadas não devem mascarar o erro silenciosamente.
* ❌ **NÃO realizar I/O síncrono e desbufferizado linha por linha:** Ler ou escrever JSONL ou registros no disco sem buffers gera milhares de chamadas de sistema desnecessárias e degrada criticamente a latência.
