# Documentação de Arquitetura e Planejamento: Adaptive Storage Engine (Rust / LSM-Tree)

Este documento estabelece as especificações, regras, padrões de comunicação e o planejamento técnico detalhado para a implementação do trabalho da disciplina de **Estruturas de Dados 2 (ED2)**.

---

## 1. Visão Geral e Regras do Trabalho

### 1.1 Objetivo
Construir um **mecanismo de armazenamento persistente chave-valor** (*storage engine*) em disco, capaz de:
* Armazenar, recuperar, remover e consultar registros com chaves `uint64` (`u64` em Rust) e valores arbitrários de tamanho variável.
* Manter corretude após encerramento normal e reinicialização.
* Resistir a interrupções abruptas (*crashes*) sem corromper dados já confirmados.
* Operar com conjuntos de dados significativamente maiores do que a memória RAM disponibilizada.
* Responder workloads orientados a lotes via formato padronizado.

### 1.2 Regras e Restrições Formais
1. **Sem DBMSs Prontos:** É estritamente proibido delegar o armazenamento a soluções prontas (como SQLite, RocksDB, LevelDB, Sled, DuckDB, etc.).
2. **Bibliotecas Permitidas:** Bibliotecas auxiliares de propósito geral (CLI parsing, serialização JSON, cálculo de hash/CRC, coleções) são permitidas, desde que a lógica de armazenamento, paginação, persistência e indexação seja de autoria do grupo.
3. **Chaves e Valores:**
   * Chave: inteiro sem sinal de 64 bits (`uint64` / `u64` em Rust).
   * Valor: string / sequência de bytes arbitrários de tamanho variável (em UTF-8 no protocolo JSON).
   * Semântica de escrita: um `PUT` para uma chave existente substitui o valor anterior.
4. **Semântica de Deleção:** Um `DELETE` remove logicamente ou fisicamente a chave (em LSM-Tree, usando marcador *tombstone*).
5. **Critério Fundamental de Corretude:** **Corretude é pré-requisito para desempenho.** Uma resposta incorreta anula a pontuação de desempenho no respectivo workload, mas uma falha isolada não implica automaticamente nota zero em toda a entrega.
6. **Datasets Superiores à Memória:** O engine deve ser capaz de operar com conjuntos de dados significativamente maiores do que o limite de memória RAM disponibilizado.
7. **Liberdade Arquitetural com Justificativa:** Não há obrigação de reproduzir uma estrutura única específica; o grupo é livre para escolher sua arquitetura (ex.: LSM-Tree), devendo justificar suas escolhas e demonstrar experimentalmente a robustez e adequação a diferentes cargas.

### 1.3 Formação de Equipe e Entregáveis Obrigatórios
* **Composição do Grupo:** Grupos de até 3 alunos.
* **Componentes de Cada Entrega:** Cada uma das três entregas cumulativas deve conter obrigatoriamente:
  1. Código-fonte completo da solução.
  2. Artefato de execução solicitado (executável `/engine` ou imagem Docker correspondente).
  3. Documentação técnica exigida.
  4. Relatório e resultados da análise experimental.

### 1.4 Objetivos de Aprendizagem
* Projetar estruturas de dados voltadas à memória secundária (disco/SSD).
* Implementar persistência, recuperação pós-crash e verificação de integridade.
* Administrar rigorosamente o uso de memória RAM e espaço em disco.
* Escolher e justificar índices e organizações de arquivos físicos.
* Medir desempenho computacional de maneira científica e reproduzível.
* Analisar compromissos (*trade-offs*) entre tempo, espaço, escrita, leitura e robustez.

### 1.5 Glossário do Domínio
| Termo | Descrição breve |
| :--- | :--- |
| **ACID** | Propriedades desejáveis para transações: atomicidade, consistência, isolamento e durabilidade. Não será exigida uma implementação ACID completa. |
| **Benchmark** | Experimento controlado usado para medir desempenho, consumo de recursos ou robustez. |
| **Baseline** | Implementação de referência usada como ponto de comparação; não é necessariamente a solução mais rápida. |
| **Cache / buffer** | Área em RAM usada para manter temporariamente dados do disco e reduzir acessos repetidos. |
| **Chave–valor** | Organização de dados em que uma chave identifica um valor associado. |
| **Crash** | Interrupção abrupta de um programa ou ambiente de execução, sem encerramento normal. |
| **Dataset** | Conjunto de dados manipulado pelos testes. |
| **Engine** | Programa responsável por organizar, armazenar, localizar e recuperar dados persistentes. |
| **Índice** | Estrutura auxiliar que acelera a localização de dados. |
| **JSON Lines (JSONL)** | Arquivo de texto com um objeto JSON por linha. |
| **Persistência** | Propriedade de manter dados após o programa ser encerrado ou reiniciado. |
| **RAM** | Memória principal do computador: rápida, mas limitada e não persistente. |
| **Recuperação** | Procedimento que restaura um estado consistente após um crash. |
| **SCAN** | Consulta que retorna registros cujas chaves pertencem a um intervalo, em ordem de chave. |
| **Seed** | Valor inicial de um gerador pseudorrandômico; permite reproduzir um workload. |
| **Throughput** | Quantidade de operações concluídas por unidade de tempo. |
| **uint64** | Inteiro sem sinal de 64 bits; tipo oficial das chaves. |
| **Workload** | Sequência de operações que simula uma forma de uso do engine. |
| **Zipf** | Distribuição em que poucas chaves são muito acessadas e muitas são pouco acessadas. |

---

## 2. Interface de Linha de Comando (CLI) e Protocolo

O executável (que será testado localmente e potencialmente em contêiner Docker Linux `amd64` sob o path `/engine`) deve implementar a seguinte interface de linha de comando:

### 2.1 Comandos CLI

```bash
# 1. Inicializa o diretório de dados com os arquivos e metadados necessários
/engine init --data-dir /caminho/do/diretorio

# 2. Executa um workload lendo um arquivo JSONL de entrada e gerando o resultado JSONL
/engine run --data-dir /caminho/do/diretorio --input /caminho/workload.jsonl --output /caminho/results.jsonl

# 3. Executa verificação de integridade nos dados persistidos
/engine verify --data-dir /caminho/do/diretorio

# 4. Exibe metadados, identificação da equipe e recursos implementados pelo engine
/engine describe
```

### 2.2 Protocolo JSON Lines (JSONL)

Todas as requisições e respostas utilizam **um objeto JSON por linha (UTF-8)**. Toda resposta deve preservar rigorosamente o campo `id` da requisição correspondente.

#### Formato das Operações (Entrada):
```json
{"id": 1, "op": "put", "key": 91, "value": "abc"}
{"id": 2, "op": "get", "key": 91}
{"id": 3, "op": "delete", "key": 91}
{"id": 4, "op": "scan", "start": 0, "end": 100}
```

#### Formato das Respostas (Saída):
```json
{"id": 1, "status": "ok"}
{"id": 2, "status": "ok", "value": "abc"}
{"id": 3, "status": "ok"}
{"id": 4, "status": "ok", "records": [{"key": 91, "value": "abc"}]}
{"id": 10, "status": "not_found"}
```

#### Regras do comando `SCAN`:
* O intervalo é **inclusivo**: `start <= key <= end`.
* Os registros retornados no array `records` devem estar **estritamente ordenados por chave** em ordem crescente (`key`).
* Não pode haver chaves duplicadas no retorno.
* Chaves removidas (*tombstones*) não devem aparecer.

---

## 3. Arquitetura Escolhida: Trilha LSM-Tree em Rust

### 3.1 Por que Rust?
* **Gerenciamento de memória determinístico:** Sem pausas de Garbage Collector (GC), permitindo respeitar rigorosamente tetos de RAM impostos pelos benchmarks.
* **Segurança e I/O de baixo nível:** Acesso direto e seguro a manipulação de arquivos, buffers (`BufReader`, `BufWriter`) e chamadas de sincronização com disco (`fsync`).
* **Ecossistema:** Crates robustas e eficientes para JSON (`serde` / `serde_json`) e verificação de integridade (`crc32fast`).

### 3.2 Visão Geral da Arquitetura LSM (Log-Structured Merge-Tree)
A arquitetura se baseia em transformar acessos aleatórios de escrita em gravações sequenciais e contínuas em disco, com dados organizados em:
1. **MemTable:** Tabela de escrita em memória RAM (usando `BTreeMap`), ordenada por chave.
2. **WAL (Write-Ahead Log):** Registro em disco append-only que garante durabilidade imediata antes de responder ao cliente.
3. **SSTables (Sorted String Tables):** Arquivos imutáveis no disco gerados quando a MemTable atinge a capacidade máxima, divididos em blocos de dados ordenados e blocos de índices.
4. **K-Way Merge Iterator:** Mecanismo com fila de prioridade (`BinaryHeap`) para unificar MemTable e SSTables em leituras e scans.

```
       [PUT / DELETE]
             │
      ┌──────┴──────┐
      ▼             ▼
   [WAL]       [MemTable] (BTreeMap em RAM)
 (em disco)         │
                    │  (Flush quando cheia)
                    ▼
               [SSTable 0] (Imutável no disco: Dados + Índice + CRC)
               [SSTable 1]
                    │
                    ▼  (Compaction na Entrega 3)
               [SSTables Consolidadas]
```

---

## 4. Planejamento Detalhado por Etapa

A entrega é cumulativa. A estrutura montada na Etapa 1 servirá de fundação direta para as Etapas 2 e 3.

```
┌─────────────────────────────────────────────────────────────┐
│ ENTREGA 1 (Peso 25%)                                        │
│ • PUT, GET, DELETE                                          │
│ • Append-only Log / WAL com CRC32                           │
│ • Recuperação de Crash & Verificação de Integridade         │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ ENTREGA 2 (Peso 30%)                                        │
│ • SCAN ordenado por chave (start <= key <= end)             │
│ • MemTable + Flush para SSTables (Dados ordenados + Índice) │
│ • K-Way Merge Iterator para busca e scan multi-nível        │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ ENTREGA 3 (Peso 45%)                                        │
│ • Filtros de Bloom por SSTable                              │
│ • Block Cache LRU/2Q para cargas Zipf / Hotspots            │
│ • Compaction de SSTables (remover lixo e tombstones)        │
│ • Adaptação dinâmica a diferentes padrões de workload      │
└─────────────────────────────────────────────────────────────┘
```

---

### Etapa 1 — Persistent Storage Engine (Peso: 25%)

#### Escopo e Requisitos:
* Implementar operações: `PUT(key, value)`, `GET(key)`, `DELETE(key)`.
* Suporte a valores de tamanho variável.
* Persistência em disco: os dados devem sobreviver à reinicialização do processo.
* Verificação básica de integridade e recuperação pós-crash (quedas abruptas de energia ou SIGKILL).
* Comandos CLI: `init`, `run`, `verify`, `describe`.

#### Decisões de Implementação (Rust):
1. **Formato do Registro Binário no Log (Append-Only):**
   Cada entrada gravada no log de dados / WAL terá o seguinte layout:
   ```
   +---------------+------------+--------------+---------------+-----------------+
   | CRC32 (4 B)   | Flags (1B) | Key (8 B)    | ValLen (4 B)  | Value (N Bytes) |
   +---------------+------------+--------------+---------------+-----------------+
   ```
   * `CRC32` (`u32`): Checksum calculado sobre os campos `Flags + Key + ValLen + Value`.
   * `Flags` (`u8`): `0x01` para registro ativo (`PUT`), `0x02` para registro removido (*tombstone* / `DELETE`).
   * `Key` (`u64` little-endian): Chave de 64 bits.
   * `ValLen` (`u32` little-endian): Tamanho do valor em bytes (0 se for tombstone).
   * `Value`: Sequência de bytes em UTF-8 do valor.

2. **Índice em Memória (Estilo Bitcask):**
   * Estrutura: `HashMap<u64, RecordOffset>` ou `BTreeMap<u64, RecordOffset>`.
   * Guarda o ponteiro do byte (`offset`) e tamanho do registro no arquivo de log.
   * `GET`: Busca o offset no mapa em memória $\to$ lê diretamente o registro no disco via `seek` / `read_exact` $\to$ valida o CRC32 $\to$ retorna o valor.
   * `PUT` / `DELETE`: Escreve no fim do arquivo via append $\to$ executa `flush`/`sync` $\to$ atualiza o ponteiro no mapa.

3. **Recuperação e Tolerância a Crash:**
   * Ao inicializar (`/engine run`), o arquivo de log é lido sequencialmente do início ao fim.
   * Cada registro tem seu CRC32 recalculado e validado.
   * Se um registro parcial for detectado no fim do arquivo (indicando queda durante a escrita de um `PUT`), o arquivo é truncado de forma segura no último registro íntegro.
   * Registros com flag de *tombstone* removem a chave do mapa em memória.

4. **Comando `verify`:**
   * Percorre todo o arquivo em disco conferindo os offsets e checksums. Se houver divergência, aponta inconsistência e falha controladamente com código de erro claro.

---

### Etapa 2 — Indexed Storage Engine (Peso: 30%)

#### Escopo e Requisitos:
* Implementar operação: `SCAN(start, end)`.
* Retorno estritamente ordenado por chave (`start <= key <= end`), sem duplicatas e sem chaves deletadas.
* Desempenho eficiente em memória secundária tanto para buscas pontuais (`GET`) quanto para buscas em intervalo (`SCAN`).
* Cumprimento de limites rigorosos de memória RAM (o dataset não cabe todo na memória).

#### Decisões de Implementação (Rust):
1. **Transição para LSM-Tree Estruturada:**
   * O mapa em memória agora se torna a **MemTable** (`BTreeMap<u64, Option<Vec<u8>>>`), onde `None` representa *tombstone*.
   * Um limite de memória (ex: 4 MB a 16 MB) é configurado para a MemTable.
   * As escritas continuam indo para o **WAL** sequencial para garantir durabilidade imediata.

2. **Flush para SSTables (Sorted String Tables):**
   * Quando a MemTable atinge o limite configurado:
     1. Itera sobre o `BTreeMap` (que já entrega os registros ordenados por chave).
     2. Grava um novo arquivo de SSTable imutável em disco (ex: `sstable_0001.sst`).
     3. Gera um **Índice Esparso** (*Sparse Index*) gravado no rodapé da SSTable ou em arquivo companheiro `.idx`: guarda `(primeira_chave_do_bloco, offset_do_bloco)` a cada bloco de 4 KB.
     4. Zera a MemTable e trunca/inicia um novo WAL.

3. **Formato da SSTable:**
   ```
   ┌────────────────────────────────────────────────────────┐
   │ Bloco de Dados 0 (registros ordenados com CRC)        │
   │ Bloco de Dados 1 (registros ordenados com CRC)        │
   │ ...                                                    │
   │ Bloco de Dados N                                       │
   ├────────────────────────────────────────────────────────┤
   │ Bloco de Índice Esparso (Offset e Chave Inicial/Bloco) │
   ├────────────────────────────────────────────────────────┤
   │ Rodapé / Footer (Offsets de metadados + Magic Bytes)   │
   └────────────────────────────────────────────────────────┘
   ```

4. **Execução de `SCAN(start, end)` (K-Way Merge):**
   * Coleta iteradores para a faixa `[start, end]`:
     * 1 iterador sobre a MemTable atual.
     * 1 iterador para cada SSTable existente que intercepte a faixa `[start, end]` (consultando os metadados de `min_key` e `max_key` de cada SSTable).
   * Utiliza um `BinaryHeap` (Min-Heap) em Rust que compara `(chave, timestamp_ou_versão)`:
     * Chaves iguais são deduplicadas pegando a versão mais recente (a versão mais jovem prevalece).
     * Se a versão mais recente for um *tombstone*, o registro é descartado.
     * Os registros válidos são emitidos em ordem ascendente estrita.

---

### Etapa 3 — Adaptive Storage Engine (Peso: 45%)

#### Escopo e Requisitos:
* Desempenho elevado diante de múltiplos tipos de workloads desconhecidos e dinâmicos:
  * Leituras intensivas (*Read-heavy*), escritas intensivas (*Write-heavy*).
  * Distribuição uniforme, sequencial, reversa, Zipf / Hotspots e clustered.
  * Scans longos e curtos.
  * Restrições severas de memória RAM e de espaço em disco.
* Demonstrar evolução arquitetural experimental com dados e métricas concretas.

#### Decisões de Implementação (Rust):
1. **Filtros de Bloom (Bloom Filters):**
   * Cada SSTable gerada inclui um Filtro de Bloom serializado.
   * Antes de realizar I/O em disco para uma SSTable durante um `GET`, consulta-se o filtro:
     * Se o filtro retornar `false`, a chave definitivamente não está naquela SSTable, economizando I/O.
     * Reduz drasticamente a latência de leituras pontuais em chaves inexistentes ou atualizadas.

2. **Block Cache Adaptativo:**
   * Implementação de cache de blocos de disco em memória RAM (usando LRU ou algoritmo 2Q).
   * Workloads com distribuição **Zipf** ou **Hotspot** terão seus blocos quentes servidos diretamente da RAM sem tocar o disco.

3. **Mecanismo de Compactação (Compaction):**
   * À medida que novas SSTables são criadas, chaves sobrescritas e tombstones acumulam espaço em disco e degradam o desempenho de leitura.
   * Implementação de compactação (ex: *Size-Tiered* ou *Leveled Compaction* simplificada):
     * Lê 2 ou mais SSTables simultaneamente via K-way merge.
     * Remove versões antigas e tombstones expirados.
     * Escreve uma nova SSTable consolidada e remove os arquivos antigos de forma atômica.

4. **Adaptação Dinâmica a Padrões de Carga:**
   * **Detecção de Scan Contínuo / Prefetching:** Se um `SCAN` solicitar um intervalo grande, o engine faz leitura antecipada em buffer dos blocos sequenciais seguintes da SSTable.
   * **Compressão Opcional de Blocos:** Compressão de blocos de dados com LZ4/Snappy para reduzir pressão de disco e aumentar throughput de leitura sob I/O bound.
   * **Ajuste de Buffer:** Ajuste adaptativo do tamanho da MemTable baseado na proporção observada entre leituras e escritas.

---

## 5. Estrutura Proposta para o Projeto em Rust

```
storage-engine/
├── Cargo.toml
├── Dockerfile                   # Ambiente oficial compatível com Linux amd64
├── src/
│   ├── main.rs                  # Ponto de entrada da CLI (/engine)
│   ├── cli.rs                   # Parser de argumentos (init, run, verify, describe)
│   ├── protocol.rs              # Serialização/Deserialização de JSONL (serde)
│   ├── engine.rs                # Fachada principal do Storage Engine
│   ├── wal/                     # Write-Ahead Log e formato binário de registros
│   │   ├── mod.rs
│   │   ├── record.rs            # Codificação do registro, flags e CRC32
│   │   └── reader.rs            # Varredura sequencial e recuperação de crash
│   ├── memtable/                # Estruturas em memória (BTreeMap)
│   │   └── mod.rs
│   ├── sstable/                 # SSTables para Entrega 2 e 3
│   │   ├── mod.rs
│   │   ├── block.rs             # Blocos de dados de tamanho fixo
│   │   ├── builder.rs           # Criação e flush de SSTables
│   │   ├── reader.rs            # Leitura com busca binária no índice
│   │   └── merge_iterator.rs    # K-Way Merge para o comando SCAN
│   ├── adaptive/                # Otimizações para Entrega 3
│   │   ├── bloom.rs             # Filtro de Bloom
│   │   ├── cache.rs             # Block cache LRU
│   │   └── compaction.rs        # Fusão e limpeza de SSTables
│   └── error.rs                 # Tipos de erro do sistema
└── tests/                       # Testes de integração locais
    ├── crash_recovery_test.rs
    ├── scan_ordering_test.rs
    └── workload_runner.rs
```

### Dependências Recomendadas (`Cargo.toml`):
* `clap` (com feature `derive`): Tratamento robusto e simples dos comandos CLI.
* `serde` e `serde_json`: Manipulação ultra rápida e tipada do protocolo JSONL.
* `crc32fast`: Cálculo de CRC32 otimizado por hardware (SIMD) para integridade dos blocos.
* `byteorder`: Garantia de ordem de bytes explícita (Little Endian) na serialização binária.

---

## 6. Metodologia de Avaliação, ED2Bench e Famílias de Workloads

### 6.1 Ferramenta ED2Bench e Oracle Independente
* A avaliação automatizada do trabalho será conduzida utilizando as ferramentas oficiais **ED2Bench**.
* O ED2Bench executará os comandos do engine e confrontará cada uma das respostas geradas contra um **oráculo independente**.
* **Modalidade de Execução:** Poderá ser centralizada, local assistida ou híbrida (a ser definida pela disciplina antes da primeira entrega).

### 6.2 Workloads de Desenvolvimento e Avaliação
* **Workloads Públicos:** Conjuntos de teste disponibilizados previamente para o grupo utilizar durante o desenvolvimento e depuração local.
* **Workloads Privados:** Conjuntos de teste utilizados na avaliação final. Os workloads privados poderão variar dinamicamente dentro de faixas pré-estabelecidas:
  * Semente pseudorrandômica (*seed*);
  * Tamanho total do dataset e cardinalidade de chaves;
  * Número total de operações;
  * Distribuição dos acessos;
  * Comprimento e seletividade dos comandos `SCAN`.

### 6.3 Famílias de Testes e Cenários de Stress
O engine será submetido a múltiplas famílias de testes projetadas para estressar diferentes aspectos da arquitetura:
* **Padrões de Acesso:** Leituras intensivas (*read-heavy*), escritas intensivas (*write-heavy*) e atualizações frequentes de chaves existentes.
* **Distribuições Estatísticas:** Uniforme, estritamente sequencial, sequencial reversa, **Zipf** (poucas chaves muito acessadas, cauda longa pouco acessada), *hotspots* concentrados e *clustered*.
* **Tamanhos e Cargas:** Registros com valores de grande porte (*large values*), cargas mistas com transições dinâmicas (*mudança de fase*).
* **Restrições Extremas:** Operação sob limites rigorosos de memória RAM (onde o dataset não cabe na memória) e pressão de armazenamento em disco.
* **Resiliência:** Interrupção abrupta (*crash* / SIGKILL / falta de energia simulada) e validação imediata da integridade e consistência dos dados após religamento.

### 6.4 Critérios de Avaliação e Regras de Pontuação
Entre os aspectos pontuados estão:
1. **Correção Funcional:** Respostas exatas para `PUT`, `GET`, `DELETE` e `SCAN`.
2. **Persistência e Recuperação:** Integridade dos dados após encerramento normal e recuperação correta após *crash*.
3. **Ordenação e Ausência de Lixo:** Scans estritamente ordenados, sem duplicatas e sem vazamento de *tombstones*.
4. **Integridade de Valores:** Verificação física e lógica sem corrupção de bytes.
5. **Uso Eficiente de Recursos:** Consumo de memória RAM rigorosamente dentro dos tetos estabelecidos e controle do espaço em disco.
6. **Robustez e Arquitetura:** Coerência técnica entre os componentes implementados e a justificativa documental.
7. **Análise Experimental:** Qualidade metodológica das medições de desempenho, throughput e latência.

> **Regra Fundamental de Avaliação:** **Correção é pré-requisito para desempenho.** Uma execução com resposta incorreta não receberá pontuação de desempenho naquele workload. No entanto, uma falha isolada em um workload específico **não implica automaticamente nota zero** em toda a entrega.

---

## 7. Diretrizes para Uso de Ferramentas de IA (AI-Assisted Engineering)

* **Permissão Ampla:** O uso de ferramentas de Inteligência Artificial generativa é **livremente autorizado** para programação, depuração, estudo, documentação, análise de resultados e apoio ao projeto.
* **Responsabilidade Integral:** A responsabilidade total pela correção, robustez, desempenho e compreensão profunda do sistema permanece **integralmente com o grupo**.
* **Capacidade de Defesa Técnica:** O grupo deve ser plenamente capaz de explicar, defender e justificar qualquer decisão técnica, estrutura de dados ou algoritmo adotado, além de interpretar criticamente os resultados de seus próprios experimentos.
* **Seção Obrigatória no Relatório Final:** Na entrega final do projeto (Entrega 3), deverá constar uma seção obrigatória intitulada **"AI-Assisted Engineering"**, detalhando como as ferramentas de IA foram integradas ao fluxo de desenvolvimento e análise da equipe.

---

## 8. Acompanhamento de Publicações Futuras da Disciplina

Para o planejamento das próximas etapas, a equipe deve acompanhar as seguintes definições que serão divulgadas pelo corpo docente:
1. Calendário oficial e regras formais de submissão de cada etapa;
2. Especificação formal detalhada do protocolo e esquemas JSON (*JSON Schemas*);
3. Modalidade final de submissão e execução da avaliação (incluindo eventual adoção de imagem Docker);
4. Versão oficial das ferramentas **ED2Bench** e instruções para execução no ambiente local;
5. Conjuntos de workloads públicos e scripts de benchmark;
6. Limites finais de recursos (tetos de RAM, limites de disco) e rubrica detalhada de pontuação;
7. Tutorial completo do ambiente de submissão.

---

## 9. Checklist de Qualidade e Boas Práticas

- [ ] **Sincronização com Disco (`fsync`):** Toda operação de escrita crítica deve ser sincronizada apropriadamente antes de confirmar `status: "ok"`, garantindo recuperação em caso de crash.
- [ ] **Tratamento de I/O Bufferizado:** Usar `BufReader` e `BufWriter` para a leitura e escrita do JSONL para evitar syscalls por linha.
- [ ] **Limites de Memória:** Nunca carregar o arquivo de dados inteiro na memória RAM. Manter apenas o índice ou a MemTable limitada.
- [ ] **Validação com Oráculo Local:** Criar um script de testes em Python ou Rust com um `HashMap` simples em memória para rodar contra o engine e comparar saídas JSONL byte a byte.
- [ ] **Registro de IA (AI-Assisted Engineering):** Documentar progressivamente as decisões arquiteturais e utilizações de IA para consolidação no relatório final.

