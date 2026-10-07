# ADR-003: Gravação Append-Only e Durabilidade Física do WAL

- **Status:** Aceito
- **Data:** 2026-10-07

---

## 1. Contexto e Problema

O Write-Ahead Log (WAL) é a fundação para a garantia ACID de Durabilidade (**D**) no mecanismo de armazenamento da Etapa 1. Toda operação confirmada (`PUT` ou `DELETE`) deve ser persistida fisicamente no disco antes de retornar a resposta ao cliente (`status: "ok"`). 

Os desafios centrais de engenharia incluem:
1. **Minimizar syscalls de I/O:** Escritas não bufferizadas geram sobrecarga excessiva devido ao volume de chamadas de sistema (`write(2)`).
2. **Durabilidade física garantida:** O buffer em espaço de usuário (`BufWriter`) e os caches de página do sistema operacional não garantem que os dados estejam fisicamente na mídia magnética ou de estado sólido (SSD). Uma queda de energia antes do sync causaria perda de dados confirmados.
3. **Rastreamento de Deslocamento (*Offsets*):** O índice em memória (*Bitcask style*) necessita saber o offset exato onde cada registro se inicia em bytes para atender consultas `GET` em $O(1)$ sem varrer o arquivo.
4. **Capacidade de Truncamento:** O leitor de inicialização (*crash recovery*) precisa ser capaz de truncar gravações parciais no final do log e reposicionar o escritor sem estados inconsistentes.

---

## 2. Decisão

1. **Abstração `WalWriter` (`src/wal/writer.rs`):**
   * Encapsula `BufWriter<std::fs::File>`, mantendo um buffer em memória para agregações rápidas e reduzindo trocas de contexto.
   * Mantém o campo atômico `current_offset: u64` atualizado deterministicamente com o somatório dos bytes gravados (`encoded_size = HEADER_SIZE + val_len`).
   * Padroniza o arquivo WAL com o nome `data.wal` (`WAL_FILE_NAME`) dentro de qualquer diretório de dados fornecido (`data_dir`).

2. **Garantia de Durabilidade com `fsync`:**
   * O método `sync(&mut self)` executa primeiramente `self.writer.flush()?` (esvaziando o buffer da aplicação para o kernel) e em seguida `self.writer.get_ref().sync_all()?`, disparando a chamada `fsync` que força a gravação física no meio persistente e nos metadados do inode.
   * O método utilitário `write_and_sync(&mut self, record: &LogRecord)` fornece o caminho crítico para operações que exigem durabilidade imediata antes do envio de respostas `ok`.

3. **Retorno de Localização Física:**
   * Ao gravar um registro via `write_record` ou `write_and_sync`, o método retorna a struct `RecordLocation::new(start_offset, val_len)`.
   * Para operações ativas (`PUT`), `val_len` é o tamanho do payload. Para exclusões (`DELETE` / tombstone), `val_len` é zero.

4. **Tratamento de Truncamento:**
   * O método `truncate(&mut self, new_len: u64)` descarrega o buffer, redimensiona o arquivo físico no disco via `set_len(new_len)`, reposiciona o cursor com `seek(SeekFrom::Start(new_len))`, sincroniza metadados com `sync_all()` e ajusta `current_offset = new_len`.

---

## 3. Alternativas Consideradas

* **I/O Direto (`O_DIRECT`):**
  - *Descartado:* Requer alinhamento estrito de memória e setores do disco (normalmente 4 KB / 512 B), inviabilizando registros de tamanho variável e impondo complexidade desnecessária para a Etapa 1.
* **Escrita Síncrona Desbufferizada (`File::write` direto):**
  - *Descartado:* Viola a diretriz de desempenho das regras do trabalho ("Não realizar I/O síncrono e desbufferizado linha por linha"). O `BufWriter` permite amortizar I/O quando lotes ou operações consecutivas ocorrem.
* **Múltiplos Arquivos de WAL Fragmentados:**
  - *Descartado para a Etapa 1:* Um único arquivo append-only sequencial (`data.wal`) simplifica a recuperação pós-crash e a indexação Bitcask nesta fase inicial.

---

## 4. Consequências e Compromissos

* **Positivas:**
  - Conformidade estrita com as regras do projeto: durabilidade física assegurada antes de confirmar qualquer escrita.
  - Baixa latência e alto throughput graças ao buffer em RAM do `BufWriter`.
  - Simplicidade operacional para reconstrução do índice em memória estilo Bitcask.
* **Compromissos:**
  - Toda operação unitária que chame `sync_all` incorre no custo de latência de rotação/persistência do disco (essencial para integridade ACID e tolerância a crash).
