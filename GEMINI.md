# GEMINI.md - Contexto, Arquitetura e Diretrizes Operacionais

> **Aviso Importante de Manutenção:** A seção **Estrutura Atual de Diretórios e Arquivos** deste documento **deve ser atualizada sempre que for adicionado, renomeado ou removido algum diretório ou arquivo** no projeto.

---

## ⚠️ Diretrizes Primárias para o Agente de IA (Leitura Obrigatória)

Antes de propor, planejar ou executar qualquer código, refatoração ou comando no projeto, o agente **DEVE OBRIGATORIAMENTE** consultar e seguir as diretrizes dos seguintes documentos centrais:

1. 🎯 **[docs/contexto/etapa_atual.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/etapa_atual.md)**: 
   Define a fase ativa de desenvolvimento (atualmente a **Etapa 1**), detalha a arquitetura imediata e fornece a **lista sequencial de tarefas com checkboxes** que norteia a implementação passo a passo. O agente deve seguir esta ordem e atualizar o progresso nas tarefas.

2. 📜 **[docs/contexto/regras.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/regras.md)**: 
   Estabelece os limites inegociáveis do trabalho, categorizados estritamente em:
   * **O que DEVE ser feito:** Corretude, durabilidade com `fsync`, validação de CRC32, truncamento em crash recovery, preservação de `id` no JSONL e limites de memória RAM.
   * **O que PODE ser feito:** Uso de crates auxiliares autorizadas (`clap`, `serde`, `serde_json`, `crc32fast`, `byteorder`, `tempfile`), I/O bufferizado, índices em RAM limitados.
   * **O que NÃO DEVE ser feito:** Proibição absoluta de DBMSs prontos (SQLite, RocksDB, etc.), proibição de carregar o dataset todo na RAM, não responder `ok` sem persistência em disco, não vazar tombstones.

3. 🏛️ **[docs/adr/](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/adr)**:
   **Registro Obrigatório de Decisões Arquiteturais (ADRs):** Qualquer adição substancial no projeto deve ser documentada de forma atômica em notas separadas no formato `.md` dentro deste diretório, descrevendo o contexto e justificando a escolha técnica e arquitetural adotada.

Documentação completa de referência arquitetural e acadêmica: [docs/contexto/DOCUMENTACAO_PROJETO.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/DOCUMENTACAO_PROJETO.md).

---

## 1. Contexto do Trabalho

O projeto consiste no desenvolvimento do **Adaptive Storage Engine**, trabalho prático da disciplina de **Estruturas de Dados 2 (ED2)**.

### 1.1 Objetivo Geral
Construir um **mecanismo de armazenamento persistente chave-valor** (*storage engine*) em disco, capaz de:
* Armazenar, recuperar, remover e consultar registros com chaves inteiras de 64 bits sem sinal (`uint64` / `u64`) e valores arbitrários de tamanho variável.
* Manter consistência e corretude após encerramento normal e reinicialização.
* Sobreviver a falhas abruptas (*crashes* / interrupções de energia) sem perda ou corrupção de dados já confirmados.
* Operar com conjuntos de dados significativamente maiores do que o limite de memória RAM disponibilizado.
* Processar workloads orientados a lotes via protocolo padronizado em **JSON Lines (JSONL)**.

### 1.2 Regras e Restrições Formais
* **Sem DBMSs Prontos:** É estritamente proibido o uso de bibliotecas de armazenamento prontas (ex.: SQLite, RocksDB, LevelDB, Sled, DuckDB). O mecanismo de paginação, serialização, persistência, recuperação e indexação deve ser de autoria própria.
* **Bibliotecas Permitidas:** Bibliotecas auxiliares de propósito geral (como parser de CLI, serialização JSON, cálculo de hash/CRC) são autorizadas.
* **Corretude é Prioridade:** Corretude funcional é pré-requisito mandatório para pontuação de desempenho.

### 1.3 Interface e Protocolo
* **Comandos CLI da aplicação (`/engine`):**
  * `init --data-dir <DIR>`: Inicializa o diretório de dados e metadados.
  * `run --data-dir <DIR> --input <IN.jsonl> --output <OUT.jsonl>`: Executa um lote de operações do workload.
  * `verify --data-dir <DIR>`: Valida a integridade física e lógica dos arquivos persistidos.
  * `describe`: Exibe informações da equipe e recursos implementados.
* **Protocolo JSONL:** Comunicação síncrona com operações `put`, `get`, `delete` e `scan`, preservando o campo `id` em cada resposta.

### 1.4 Entregas Cumulativas
1. **Entrega 1 (Peso 25%) — Persistent Storage Engine:** Operações `PUT`, `GET`, `DELETE`, persistência em log com CRC32, tolerância a crash e recuperação.
2. **Entrega 2 (Peso 30%) — Indexed Storage Engine:** Operação `SCAN(start, end)` com ordenação estrita, MemTable e flush para SSTables com índice esparso, K-Way Merge Iterator.
3. **Entrega 3 (Peso 45%) — Adaptive Storage Engine:** Filtros de Bloom, Block Cache em RAM (LRU/2Q), compactação de SSTables e adaptação dinâmica a diferentes padrões de carga.

---

## 2. Escolhas de Linguagem e Arquitetura

### 2.1 Linguagem: Rust
* **Controle Determinístico de Memória:** Ausência de coletor de lixo (*Garbage Collector* - GC), eliminando pausas imprevisíveis e garantindo controle rígido dos limites de RAM impostos pelos benchmarks.
* **I/O Seguro e de Baixo Nível:** Facilidade para manipular buffers (`BufReader`, `BufWriter`), operações binárias Little-Endian e sincronização pontual em disco (`fsync`).
* **Segurança e Concorrência:** O compilador do Rust garante integridade de referências e ausência de *data races* em tempo de compilação.
* **Ecossistema:** Crates de alto desempenho como `serde` / `serde_json` (para JSONL), `crc32fast` (cálculo de checksum com aceleração SIMD) e `clap` (parsing de CLI).

### 2.2 Arquitetura: LSM-Tree (Log-Structured Merge-Tree) Adaptativa
A arquitetura transforma acessos aleatórios de escrita em gravações contínuas e sequenciais em disco, evoluindo pelas três etapas:

1. **Write-Ahead Log (WAL) & Log Binário (Base Etapa 1):**
   * Formato de registro binário com cabeçalho contendo `CRC32 (4B) + Flags (1B) + Key (8B) + ValLen (4B) + Value (N Bytes)`.
   * Flags definem registros válidos (`PUT = 0x01`) ou marcadores de remoção (*tombstones* - `DELETE = 0x02`).
   * No arranque, o engine reexecuta o log, valida os CRCs e reconstrói o estado em memória (Bitcask-style), truncando gravações parciais causadas por falhas abruptas.

2. **MemTable e SSTables (Transição Etapa 2):**
   * **MemTable:** Estrutura em memória RAM (`BTreeMap<u64, Option<Vec<u8>>>`) com teto de tamanho configurável.
   * **SSTable (Sorted String Table):** Arquivo imutável gerado por *flush* ordenado, estruturado em Blocos de Dados com CRC e um Bloco de Índice Esparso (*Sparse Index*).
   * **K-Way Merge Iterator:** Min-Heap (`BinaryHeap`) que unifica a MemTable e as SSTables ativas para responder leituras e o comando `SCAN(start, end)` em ordem estritamente ascendente.

3. **Mecanismos Adaptativos (Etapa 3):**
   * **Filtros de Bloom:** Reduzem I/O evitando leituras em SSTables onde a chave comprovadamente não existe.
   * **Block Cache (LRU/2Q):** Mantém blocos quentes na RAM para acelerar leituras sob distribuição assimétrica (ex.: Zipf/Hotspots).
   * **Compactação:** Fusão em segundo plano de múltiplas SSTables para descartar chaves sobrescritas e *tombstones*, economizando espaço e acelerando consultas.

---

## 3. Estrutura Atual de Diretórios e Arquivos

```
.
├── .dockerignore
├── .gitignore
├── Cargo.toml
├── Dockerfile
├── GEMINI.md
├── docs/
│   ├── adr/
│   │   ├── 001-modulo-de-erros-e-tipos-basicos.md
│   │   ├── 002-formato-binario-e-serializacao-wal.md
│   │   ├── 003-gravacao-append-only-e-durabilidade-wal.md
│   │   ├── 004-varredura-sequencial-e-recuperacao-crash.md
│   │   ├── 005-nucleo-do-engine-e-indice-em-memoria.md
│   │   ├── 006-protocolo-json-lines.md
│   │   ├── 007-interface-de-linha-de-comando.md
│   │   ├── 008-ponto-de-entrada-principal.md
│   │   └── README.md
│   ├── contexto/
│   │   ├── DOCUMENTACAO_PROJETO.md
│   │   ├── etapa_atual.md
│   │   └── regras.md
│   └── estudo/
├── src/
│   ├── adaptive/
│   ├── memtable/
│   ├── sstable/
│   ├── wal/
│   │   ├── mod.rs
│   │   ├── reader.rs
│   │   ├── record.rs
│   │   └── writer.rs
│   ├── cli.rs
│   ├── engine.rs
│   ├── error.rs
│   ├── lib.rs
│   ├── main.rs
│   └── protocol.rs
└── tests/
```

### Detalhamento dos Componentes:
* [Cargo.toml](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/Cargo.toml): Manifesto de dependências e configuração de compilação do binário `engine`.
* [Dockerfile](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/Dockerfile): Definição de container Linux `amd64` multi-stage para compilar e expor `/engine`.
* [GEMINI.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/GEMINI.md): Contexto, arquitetura e diretrizes que direcionam o agente para as regras e a etapa atual.
* `docs/`:
  * [docs/adr/](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/adr): Registro atômico de decisões arquiteturais (**ADRs**) documentando e justificando escolhas técnicas do projeto em notas `.md`.
  * `docs/contexto/`:
    * [DOCUMENTACAO_PROJETO.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/DOCUMENTACAO_PROJETO.md): Documento mestre de especificação técnica e regras do trabalho.
    * [etapa_atual.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/etapa_atual.md): Escopo detalhado da etapa em andamento e checklist sequencial com checkboxes.
    * [regras.md](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/docs/contexto/regras.md): Delimitação clara do que PODE, DEVE e NÃO DEVE ser feito no trabalho.
  * `docs/estudo/`: Espaço reservado para anotações teóricas, rascunhos e estudos do grupo.
* [src/main.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/main.rs): Ponto de entrada executável da CLI (`/engine`).
* [src/cli.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/cli.rs): Interface de Linha de Comando (CLI com `clap`), implementando os comandos `init`, `run`, `verify` e `describe`.
* [src/lib.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/lib.rs): Ponto de entrada da biblioteca do engine (exportando módulos para CLI e testes).
* [src/engine.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/engine.rs): Núcleo do `StorageEngine` integrando o WAL com o índice em RAM (Bitcask-style) e garantindo durabilidade com `fsync`.
* [src/error.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/error.rs): Módulo de tipos de erro (`EngineError`), resultado (`Result<T>`) e tipos primitivos de domínio.
* [src/protocol.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/protocol.rs): Módulo do protocolo JSON Lines (JSONL), parsing/serialização em streaming e preservação estrita de `id`.
* **`src/wal/`**: Módulo para formato de registros binários, cálculo de CRC32, gravação em log e recuperação de crash.
  * [src/wal/mod.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/mod.rs): Ponto de entrada do módulo WAL.
  * [src/wal/reader.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/reader.rs): Estrutura `WalReader`, leitura pontual com `seek`, iterador sequencial em streaming e rotina de crash recovery com truncamento.
  * [src/wal/record.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/record.rs): Estrutura `LogRecord`, codificação Little-Endian e validação estrita de CRC32.
  * [src/wal/writer.rs](file:///home/rafael-siqueira/estudos/faculdade/trabalhos/ED2/src/wal/writer.rs): Estrutura `WalWriter`, escrita sequencial bufferizada, durabilidade com `fsync` e truncamento.
* **`src/memtable/`**: Módulo para estrutura em memória RAM (`BTreeMap`) e controle de limites de memória.
* **`src/sstable/`**: Módulo para blocos de dados, índice esparso, persistência imutável e K-Way Merge.
* **`src/adaptive/`**: Módulo para estratégias adaptativas (Filtro de Bloom, Cache LRU e Compactação).
* **`tests/`**: Diretório para suítes de testes de integração, simulação de crash e validação de workloads.
