//! Gravador append-only e gerenciador de durabilidade física do Write-Ahead Log (WAL).

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{RecordLocation, Result};
use crate::wal::record::LogRecord;

/// Nome padrão do arquivo WAL no diretório de dados da Etapa 1.
pub const WAL_FILE_NAME: &str = "data.wal";

/// Gravador sequencial (*append-only*) para o Write-Ahead Log em disco.
///
/// Mantém escrita bufferizada em memória para alta taxa de transferência,
/// com suporte a sincronização física atômica (`fsync`) e rastreamento rigoroso
/// dos deslocamentos em bytes (*offsets*) para indexação em memória.
pub struct WalWriter {
    writer: BufWriter<File>,
    path: PathBuf,
    current_offset: u64,
}

impl WalWriter {
    /// Abre ou cria o arquivo WAL no caminho especificado.
    ///
    /// Posiciona o cursor no final do arquivo existente e inicializa
    /// `current_offset` com base no tamanho físico atual em disco.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;

        let current_offset = file.metadata()?.len();
        file.seek(SeekFrom::Start(current_offset))?;

        Ok(Self {
            writer: BufWriter::new(file),
            path,
            current_offset,
        })
    }

    /// Abre ou cria o arquivo WAL padrão (`data.wal`) dentro de um diretório de dados.
    pub fn open_in_dir<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let path = dir.as_ref().join(WAL_FILE_NAME);
        Self::open(path)
    }

    /// Grava um registro serializado no buffer do WAL e incrementa o offset corrente.
    ///
    /// Retorna `RecordLocation` com o offset inicial e o tamanho do valor para indexação.
    /// Os bytes são retidos no buffer interno até que `flush()` ou `sync()` sejam executados.
    pub fn write_record(&mut self, record: &LogRecord) -> Result<RecordLocation> {
        let start_offset = self.current_offset;
        let written_bytes = record.encode(&mut self.writer)?;
        self.current_offset += written_bytes as u64;

        let val_len = if record.is_tombstone {
            0
        } else {
            record.value.len() as u32
        };

        Ok(RecordLocation::new(start_offset, val_len))
    }

    /// Descarrega o buffer em memória do `BufWriter` para o sistema operacional (`flush`).
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    /// Força o flush do buffer de memória e executa `fsync` (`sync_all`) no arquivo em disco,
    /// garantindo que os dados confirmados sobrevivam a falhas de energia ou encerramento abrupto.
    pub fn sync(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    /// Grava o registro e força imediatamente a persistência física no disco (`fsync`).
    ///
    /// Combina `write_record` e `sync` de forma atômica para operações unitárias que
    /// exigem confirmação com durabilidade física imediata.
    pub fn write_and_sync(&mut self, record: &LogRecord) -> Result<RecordLocation> {
        let location = self.write_record(record)?;
        self.sync()?;
        Ok(location)
    }

    /// Trunca o arquivo de log para um novo tamanho e reposiciona o cursor de escrita.
    ///
    /// Essencial para *crash recovery* ao descartar registros incompletos ou corrompidos
    /// no fim do arquivo antes de autorizar novas gravações.
    pub fn truncate(&mut self, new_len: u64) -> Result<()> {
        self.writer.flush()?;
        self.writer.get_ref().set_len(new_len)?;
        self.writer.seek(SeekFrom::Start(new_len))?;
        self.current_offset = new_len;
        self.writer.get_ref().sync_all()?;
        Ok(())
    }

    /// Retorna o offset atual (tamanho acumulado em bytes no WAL).
    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }

    /// Retorna a referência para o caminho do arquivo WAL.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::record::HEADER_SIZE;
    use tempfile::tempdir;

    #[test]
    fn test_wal_writer_open_new_and_write() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        assert_eq!(writer.current_offset(), 0);
        assert_eq!(writer.path(), dir.path().join(WAL_FILE_NAME));

        // 1. Grava PUT
        let rec1 = LogRecord::put(1001, b"valor_teste_1".to_vec());
        let expected_size_1 = (HEADER_SIZE + rec1.value.len()) as u64;
        let loc1 = writer.write_and_sync(&rec1).unwrap();

        assert_eq!(loc1.offset, 0);
        assert_eq!(loc1.value_len, rec1.value.len() as u32);
        assert_eq!(writer.current_offset(), expected_size_1);

        // 2. Grava DELETE (tombstone)
        let rec2 = LogRecord::delete(1001);
        let expected_size_2 = HEADER_SIZE as u64;
        let loc2 = writer.write_and_sync(&rec2).unwrap();

        assert_eq!(loc2.offset, expected_size_1);
        assert_eq!(loc2.value_len, 0);
        assert_eq!(writer.current_offset(), expected_size_1 + expected_size_2);
    }

    #[test]
    fn test_wal_writer_reopen_and_append() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("custom.wal");

        // Abre e grava primeiro registro
        let rec1 = LogRecord::put(1, b"primeiro".to_vec());
        let expected_size_1 = (HEADER_SIZE + rec1.value.len()) as u64;
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();
            let loc1 = writer.write_and_sync(&rec1).unwrap();
            assert_eq!(loc1.offset, 0);
            assert_eq!(writer.current_offset(), expected_size_1);
        }

        // Reabre o mesmo arquivo e verifica se mantém o offset
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();
            assert_eq!(writer.current_offset(), expected_size_1);

            let rec2 = LogRecord::put(2, b"segundo".to_vec());
            let loc2 = writer.write_and_sync(&rec2).unwrap();
            assert_eq!(loc2.offset, expected_size_1);
            assert_eq!(loc2.value_len, rec2.value.len() as u32);
            assert_eq!(
                writer.current_offset(),
                expected_size_1 + (HEADER_SIZE + rec2.value.len()) as u64
            );
        }

        // Valida leitura decodificando sequencialmente os registros
        let mut file = File::open(&wal_path).unwrap();
        let decoded1 = LogRecord::decode(&mut file, 0).unwrap().unwrap();
        assert_eq!(decoded1.key, 1);
        assert_eq!(decoded1.value, b"primeiro");

        let decoded2 = LogRecord::decode(&mut file, expected_size_1)
            .unwrap()
            .unwrap();
        assert_eq!(decoded2.key, 2);
        assert_eq!(decoded2.value, b"segundo");

        let decoded3 = LogRecord::decode(&mut file, writer_offset_check(&wal_path)).unwrap();
        assert!(decoded3.is_none());
    }

    fn writer_offset_check(path: &Path) -> u64 {
        std::fs::metadata(path).unwrap().len()
    }

    #[test]
    fn test_wal_writer_truncate() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        let rec1 = LogRecord::put(10, b"persistido".to_vec());
        let loc1 = writer.write_and_sync(&rec1).unwrap();
        let len_after_rec1 = writer.current_offset();

        let rec2 = LogRecord::put(20, b"registro_a_ser_truncado".to_vec());
        writer.write_and_sync(&rec2).unwrap();
        assert!(writer.current_offset() > len_after_rec1);

        // Trunca de volta para o tamanho de rec1
        writer.truncate(len_after_rec1).unwrap();
        assert_eq!(writer.current_offset(), len_after_rec1);
        assert_eq!(
            std::fs::metadata(writer.path()).unwrap().len(),
            len_after_rec1
        );

        // Grava um novo registro após o truncamento
        let rec3 = LogRecord::put(30, b"novo_registro".to_vec());
        let loc3 = writer.write_and_sync(&rec3).unwrap();
        assert_eq!(loc3.offset, len_after_rec1);

        // Lê o arquivo para verificar se rec2 sumiu e rec3 está no lugar
        let mut file = File::open(writer.path()).unwrap();
        let d1 = LogRecord::decode(&mut file, loc1.offset).unwrap().unwrap();
        assert_eq!(d1.key, 10);
        assert_eq!(d1.value, b"persistido");

        let d3 = LogRecord::decode(&mut file, loc3.offset).unwrap().unwrap();
        assert_eq!(d3.key, 30);
        assert_eq!(d3.value, b"novo_registro");
    }

    #[test]
    fn test_wal_writer_buffer_flush_and_sync() {
        let dir = tempdir().unwrap();
        let mut writer = WalWriter::open_in_dir(dir.path()).unwrap();

        let rec = LogRecord::put(42, b"conteudo".to_vec());
        let loc = writer.write_record(&rec).unwrap();
        assert_eq!(loc.offset, 0);

        // Força flush e sync
        writer.flush().unwrap();
        writer.sync().unwrap();

        let mut file = File::open(writer.path()).unwrap();
        let decoded = LogRecord::decode(&mut file, 0).unwrap().unwrap();
        assert_eq!(decoded.key, 42);
        assert_eq!(decoded.value, b"conteudo");
    }

    #[test]
    fn test_wal_writer_open_in_nested_dir() {
        let dir = tempdir().unwrap();
        let nested_dir = dir.path().join("sub").join("nested");
        let mut writer = WalWriter::open_in_dir(&nested_dir).unwrap();
        assert_eq!(writer.path(), nested_dir.join(WAL_FILE_NAME));
        let rec = LogRecord::put(100, b"teste nested".to_vec());
        let loc = writer.write_and_sync(&rec).unwrap();
        assert_eq!(loc.offset, 0);
        assert!(nested_dir.join(WAL_FILE_NAME).exists());
    }
}
