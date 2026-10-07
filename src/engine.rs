//! Núcleo do storage engine e gerenciador do índice em memória (Bitcask-style).
//!
//! Coordena a persistência append-only no Write-Ahead Log (WAL), garantindo durabilidade
//! imediata com `fsync` a cada operação confirmada e mantendo um índice em RAM
//! (`HashMap<Key, RecordLocation>`) para buscas pontuais em tempo $O(1)$.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Key, RecordLocation, Result};
use crate::wal::{LogRecord, RecoveryReport, WalReader, WalWriter};

/// Mecanismo central de armazenamento persistente chave-valor.
///
/// Combina gravação sequencial append-only em disco com indexação estritamente em memória
/// de deslocamentos físicos (`RecordLocation`). Nenhuma carga massiva de valores é mantida
/// em RAM, garantindo conformidade com os limites estritos de memória do trabalho.
pub struct StorageEngine {
    /// Diretório raiz onde os arquivos de dados e logs estão armazenados.
    data_dir: PathBuf,
    /// Índice em memória mapeando cada chave para sua localização física no arquivo de log.
    index: HashMap<Key, RecordLocation>,
    /// Gravador sequencial append-only no arquivo WAL.
    wal_writer: WalWriter,
    /// Leitor pontual com seek para atendimento a consultas GET com checagem de CRC32.
    wal_reader: WalReader,
    /// Relatório consolidado do processo de recuperação e reparo pós-crash executado na inicialização.
    recovery_report: RecoveryReport,
}

impl StorageEngine {
    /// Abre ou inicializa o storage engine no diretório especificado.
    ///
    /// # Procedimento de Inicialização:
    /// 1. Cria a árvore de diretórios do `data_dir` se ela ainda não existir.
    /// 2. Executa a varredura linear de recuperação pós-crash (`recover_in_dir`) em streaming,
    ///    validando os CRCs de cada registro e truncando com segurança eventuais gravações
    ///    parciais no fim do arquivo decorrentes de quedas súbitas de energia (`SIGKILL`).
    /// 3. Reconstrói o índice em memória (`HashMap<Key, RecordLocation>`):
    ///    - Registros `PUT` inserem a chave e sua localização física.
    ///    - Registros `DELETE` (tombstones) removem a chave do índice.
    ///    - O payload do valor (`Vec<u8>`) é descartado da RAM imediatamente.
    /// 4. Inicializa o escritor sequencial (`WalWriter`) posicionado no final do arquivo íntegro.
    /// 5. Inicializa o leitor pontual (`WalReader`) para atender a futuras consultas `GET`.
    pub fn open<P: AsRef<Path>>(data_dir: P) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();

        if !data_dir.exists() {
            fs::create_dir_all(&data_dir)?;
        }

        let mut index = HashMap::new();

        // Recupera o log e reconstrói o índice em streaming (baixo uso de memória)
        let recovery_report = WalReader::recover_in_dir(&data_dir, |entry| {
            if entry.record.is_tombstone {
                index.remove(&entry.record.key);
            } else {
                index.insert(entry.record.key, entry.location);
            }
            Ok(())
        })?;

        // Inicializa o escritor append-only e o leitor pontual
        let wal_writer = WalWriter::open_in_dir(&data_dir)?;
        let wal_reader = WalReader::open_in_dir(&data_dir)?;

        Ok(Self {
            data_dir,
            index,
            wal_writer,
            wal_reader,
            recovery_report,
        })
    }

    /// Insere ou atualiza uma chave no storage engine.
    ///
    /// Persiste o registro no WAL com cálculo de CRC32 e sincronização física obrigatória
    /// no disco (`fsync`), atualizando o índice em memória somente após confirmação da durabilidade.
    pub fn put(&mut self, key: Key, value: Vec<u8>) -> Result<RecordLocation> {
        let record = LogRecord::put(key, value);
        let location = self.wal_writer.write_and_sync(&record)?;
        self.index.insert(key, location);
        Ok(location)
    }

    /// Recupera o valor mais recente associado à chave especificada.
    ///
    /// Consulta o índice em memória em $O(1)$ para obter o deslocamento físico no WAL,
    /// posiciona o cursor do arquivo (`seek`), lê os bytes do disco e valida o CRC32.
    ///
    /// Retorna `Ok(Some(value))` caso a chave exista e esteja íntegra,
    /// ou `Ok(None)` se a chave não existir ou tiver sido excluída logicamente.
    pub fn get(&mut self, key: Key) -> Result<Option<Vec<u8>>> {
        let location = match self.index.get(&key) {
            Some(&loc) => loc,
            None => return Ok(None),
        };

        let value = self.wal_reader.read_value_at(location)?;
        Ok(Some(value))
    }

    /// Remove logicamente uma chave do storage engine gravando um marcador de exclusão (*tombstone*).
    ///
    /// O tombstone é persistido fisicamente no disco com `fsync` antes de remover a chave do
    /// índice em RAM, assegurando que exclusões confirmadas sobrevivam a crashes e reinicializações.
    ///
    /// Retorna `Ok(true)` se a chave constava no índice em memória antes da remoção,
    /// ou `Ok(false)` caso ela já não existisse.
    pub fn delete(&mut self, key: Key) -> Result<bool> {
        let record = LogRecord::delete(key);
        self.wal_writer.write_and_sync(&record)?;
        let existed = self.index.remove(&key).is_some();
        Ok(existed)
    }

    /// Verifica se a chave fornecida está presente e ativa no índice em memória.
    pub fn contains_key(&self, key: &Key) -> bool {
        self.index.contains_key(key)
    }

    /// Retorna o número de chaves ativas (não excluídas) atualmente indexadas.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Retorna `true` se o engine não contiver nenhuma chave ativa indexada.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Retorna a localização física (`RecordLocation`) de uma chave, se presente no índice.
    pub fn get_location(&self, key: &Key) -> Option<RecordLocation> {
        self.index.get(key).copied()
    }

    /// Retorna a referência para o caminho do diretório de dados gerenciado.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Retorna o relatório consolidado da recuperação pós-crash executada durante o `open`.
    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.recovery_report
    }

    /// Retorna um iterador com referências a todas as chaves ativas no índice.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.index.keys()
    }

    /// Força o flush dos buffers e a sincronização física de todos os dados pendentes no disco (`fsync`).
    pub fn sync(&mut self) -> Result<()> {
        self.wal_writer.sync()
    }

    /// Valida integralmente a integridade física de todos os registros persistidos no WAL.
    ///
    /// Percorre o arquivo de log do início ao fim calculando e validando os CRCs de cada registro.
    /// Retorna a quantidade total de registros íntegros validados ou erro caso corrupção seja encontrada.
    pub fn verify(&mut self) -> Result<usize> {
        self.wal_writer.flush()?;
        let wal_path = self.wal_writer.path();
        let (records_count, _bytes) = WalReader::verify_file(wal_path)?;
        Ok(records_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_engine_open_empty_dir() {
        let dir = tempdir().unwrap();
        let engine = StorageEngine::open(dir.path()).unwrap();

        assert_eq!(engine.len(), 0);
        assert!(engine.is_empty());
        assert_eq!(engine.recovery_report().valid_records_count, 0);
        assert!(!engine.recovery_report().truncated);
    }

    #[test]
    fn test_engine_put_and_get() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        let key1 = 100u64;
        let val1 = b"hello_world".to_vec();

        let key2 = 200u64;
        let val2 = b"segundo_valor_ed2".to_vec();

        let loc1 = engine.put(key1, val1.clone()).unwrap();
        let loc2 = engine.put(key2, val2.clone()).unwrap();

        assert_eq!(loc1.offset, 0);
        assert_eq!(loc1.value_len, val1.len() as u32);
        assert!(loc2.offset > loc1.offset);
        assert_eq!(engine.len(), 2);
        assert!(!engine.is_empty());

        // Consulta chaves existentes
        assert_eq!(engine.get(key1).unwrap(), Some(val1));
        assert_eq!(engine.get(key2).unwrap(), Some(val2));

        // Consulta chave inexistente
        assert_eq!(engine.get(999).unwrap(), None);
    }

    #[test]
    fn test_engine_put_overwrite() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        let key = 42u64;
        engine.put(key, b"versao_1".to_vec()).unwrap();
        assert_eq!(engine.get(key).unwrap(), Some(b"versao_1".to_vec()));
        assert_eq!(engine.len(), 1);

        // Sobrescreve com novo valor
        let loc2 = engine.put(key, b"versao_2_atualizada".to_vec()).unwrap();
        assert_eq!(engine.get(key).unwrap(), Some(b"versao_2_atualizada".to_vec()));
        assert_eq!(engine.len(), 1);
        assert!(loc2.offset > 0);
    }

    #[test]
    fn test_engine_delete() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        let key = 777u64;
        engine.put(key, b"temporario".to_vec()).unwrap();
        assert!(engine.contains_key(&key));
        assert_eq!(engine.get(key).unwrap(), Some(b"temporario".to_vec()));

        // Deleta chave existente
        let existed = engine.delete(key).unwrap();
        assert!(existed);
        assert!(!engine.contains_key(&key));
        assert_eq!(engine.len(), 0);
        assert_eq!(engine.get(key).unwrap(), None);

        // Deleta chave já inexistente
        let existed_again = engine.delete(key).unwrap();
        assert!(!existed_again);
        assert_eq!(engine.get(key).unwrap(), None);
    }

    #[test]
    fn test_engine_persistence_across_reopen() {
        let dir = tempdir().unwrap();

        // 1. Abre, grava dados e fecha (drop)
        {
            let mut engine = StorageEngine::open(dir.path()).unwrap();
            engine.put(1, b"val_1".to_vec()).unwrap();
            engine.put(2, b"val_2".to_vec()).unwrap();
            engine.put(3, b"val_3".to_vec()).unwrap();
            // Sobrescreve chave 2
            engine.put(2, b"val_2_atualizado".to_vec()).unwrap();
            // Deleta chave 1
            engine.delete(1).unwrap();
        }

        // 2. Reabre e verifica que o índice e os dados foram recuperados perfeitamente
        {
            let mut engine = StorageEngine::open(dir.path()).unwrap();
            assert_eq!(engine.len(), 2); // Chaves 2 e 3 ativas
            assert!(!engine.contains_key(&1));
            assert_eq!(engine.get(1).unwrap(), None);
            assert_eq!(engine.get(2).unwrap(), Some(b"val_2_atualizado".to_vec()));
            assert_eq!(engine.get(3).unwrap(), Some(b"val_3".to_vec()));

            assert_eq!(engine.recovery_report().valid_records_count, 5); // 1, 2, 3, 2_up, 1_del
            assert!(!engine.recovery_report().truncated);

            // Continua operando normalmente
            engine.put(4, b"val_4".to_vec()).unwrap();
            assert_eq!(engine.get(4).unwrap(), Some(b"val_4".to_vec()));
            assert_eq!(engine.len(), 3);
        }
    }

    #[test]
    fn test_engine_crash_recovery_truncation() {
        let dir = tempdir().unwrap();

        // 1. Grava 3 registros íntegros
        {
            let mut engine = StorageEngine::open(dir.path()).unwrap();
            engine.put(10, b"dez".to_vec()).unwrap();
            engine.put(20, b"vinte".to_vec()).unwrap();
            engine.put(30, b"trinta".to_vec()).unwrap();
        }

        // 2. Simula crash corrompendo/truncando o arquivo no fim (bytes parciais)
        let wal_path = dir.path().join(crate::wal::WAL_FILE_NAME);
        {
            let mut file = OpenOptions::new()
                .write(true)
                .append(true)
                .open(&wal_path)
                .unwrap();
            // Adiciona 7 bytes de lixo simulando queda de energia no meio da escrita de um cabeçalho
            file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03])
                .unwrap();
            file.sync_all().unwrap();
        }

        // 3. Reabre o engine: deve recuperar registros íntegros e truncar o lixo no fim
        {
            let mut engine = StorageEngine::open(dir.path()).unwrap();
            assert_eq!(engine.recovery_report().valid_records_count, 3);
            assert!(engine.recovery_report().truncated);
            assert_eq!(engine.recovery_report().truncated_bytes, 7);

            // Chaves anteriores continuam 100% íntegras
            assert_eq!(engine.get(10).unwrap(), Some(b"dez".to_vec()));
            assert_eq!(engine.get(20).unwrap(), Some(b"vinte".to_vec()));
            assert_eq!(engine.get(30).unwrap(), Some(b"trinta".to_vec()));
            assert_eq!(engine.len(), 3);

            // Novos registros podem ser gravados a partir do ponto truncado
            engine.put(40, b"quarenta".to_vec()).unwrap();
            assert_eq!(engine.get(40).unwrap(), Some(b"quarenta".to_vec()));
            assert_eq!(engine.len(), 4);
        }

        // 4. Reabre mais uma vez: arquivo agora deve estar limpo e sem truncamentos pendentes
        {
            let mut engine = StorageEngine::open(dir.path()).unwrap();
            assert_eq!(engine.recovery_report().valid_records_count, 4);
            assert!(!engine.recovery_report().truncated);
            assert_eq!(engine.get(40).unwrap(), Some(b"quarenta".to_vec()));
            assert_eq!(engine.verify().unwrap(), 4);
        }
    }

    #[test]
    fn test_engine_verify_intact_and_sync() {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::open(dir.path()).unwrap();

        for i in 1..=50 {
            engine.put(i, format!("valor_{i}").into_bytes()).unwrap();
        }

        assert_eq!(engine.verify().unwrap(), 50);
        engine.sync().unwrap();
    }
}
