//! Suíte de Testes Automatizados da Etapa 1: Recuperação Pós-Crash e Tolerância a Falhas.
//!
//! Valida a capacidade do StorageEngine de resistir a interrupções abruptas (crashes de energia,
//! SIGKILL), truncando com segurança gravações parciais no final do WAL, restaurando registros
//! íntegros confirmados e recusando corrupções físicas no meio do log.

use std::fs::{self, OpenOptions};
use std::io::Write;
use storage_engine::cli::handle_verify;
use storage_engine::engine::StorageEngine;
use storage_engine::error::EngineError;
use storage_engine::wal::record::{calculate_crc, LogRecord, FLAG_PUT};
use storage_engine::wal::WAL_FILE_NAME;
use tempfile::tempdir;

#[test]
fn test_crash_recovery_partial_header_at_eof_truncates_and_recovers() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // 1. Grava 3 registros íntegros
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(1, b"primeiro".to_vec()).unwrap();
        engine.put(2, b"segundo".to_vec()).unwrap();
        engine.put(3, b"terceiro".to_vec()).unwrap();
        assert_eq!(engine.len(), 3);
    }

    let intact_size = fs::metadata(&wal_path).unwrap().len();

    // 2. Simula crash abrupto adicionando 7 bytes incompletos (menor que HEADER_SIZE=17)
    {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&wal_path)
            .expect("Falha ao abrir WAL para appending");
        file.write_all(&[0x12, 0x34, 0x56, 0x78, 0x01, 0xAA, 0xBB])
            .unwrap();
        file.sync_all().unwrap();
    }

    let corrupted_size = fs::metadata(&wal_path).unwrap().len();
    assert_eq!(corrupted_size, intact_size + 7);

    // 3. Reabre o engine: deve detectar o truncamento e reparar o WAL
    {
        let mut engine = StorageEngine::open(dir_path).expect("StorageEngine deve se recuperar do crash");
        let report = engine.recovery_report();

        assert!(report.truncated, "O relatório deve acusar truncamento");
        assert_eq!(report.valid_records_count, 3);
        assert_eq!(report.valid_offset, intact_size);
        assert_eq!(report.truncated_bytes, 7);

        // Verifica que o arquivo físico no disco foi realmente truncado
        let repaired_size = fs::metadata(&wal_path).unwrap().len();
        assert_eq!(repaired_size, intact_size);

        // Registros íntegros devem continuar acessíveis
        assert_eq!(engine.get(1).unwrap(), Some(b"primeiro".to_vec()));
        assert_eq!(engine.get(2).unwrap(), Some(b"segundo".to_vec()));
        assert_eq!(engine.get(3).unwrap(), Some(b"terceiro".to_vec()));

        // Inserção de novo registro pós-recuperação deve suceder normalmente
        engine.put(4, b"quarto_pos_crash".to_vec()).unwrap();
        assert_eq!(engine.len(), 4);
        assert_eq!(engine.get(4).unwrap(), Some(b"quarto_pos_crash".to_vec()));
    }

    // 4. Nova reabertura deve encontrar o arquivo 100% limpo sem truncamento
    {
        let engine = StorageEngine::open(dir_path).unwrap();
        let report = engine.recovery_report();
        assert!(!report.truncated);
        assert_eq!(report.valid_records_count, 4);
    }
}

#[test]
fn test_crash_recovery_partial_payload_at_eof_truncates_and_recovers() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // 1. Grava 2 registros válidos
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(10, b"valor_dez".to_vec()).unwrap();
        engine.put(20, b"valor_vinte".to_vec()).unwrap();
    }

    let intact_size = fs::metadata(&wal_path).unwrap().len();

    // 2. Simula crash onde o cabeçalho foi escrito completamente anunciando 50 bytes de valor,
    // mas apenas 15 bytes de valor foram gravados antes da falha de energia
    {
        let key = 30u64;
        let val_len = 50u32;
        let partial_payload = b"apenas_15_bytes";
        let crc = calculate_crc(FLAG_PUT, key, val_len, partial_payload);

        let mut corrupted_entry = Vec::new();
        corrupted_entry.extend_from_slice(&crc.to_le_bytes());
        corrupted_entry.push(FLAG_PUT);
        corrupted_entry.extend_from_slice(&key.to_le_bytes());
        corrupted_entry.extend_from_slice(&val_len.to_le_bytes());
        corrupted_entry.extend_from_slice(partial_payload);

        let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
        file.write_all(&corrupted_entry).unwrap();
        file.sync_all().unwrap();
    }

    // 3. Reabertura do engine
    {
        let mut engine = StorageEngine::open(dir_path).expect("Falha ao recuperar de payload truncado");
        let report = engine.recovery_report();

        assert!(report.truncated);
        assert_eq!(report.valid_records_count, 2);
        assert_eq!(report.valid_offset, intact_size);

        // O arquivo físico deve estar truncado no tamanho intacto
        let repaired_size = fs::metadata(&wal_path).unwrap().len();
        assert_eq!(repaired_size, intact_size);

        assert_eq!(engine.get(10).unwrap(), Some(b"valor_dez".to_vec()));
        assert_eq!(engine.get(20).unwrap(), Some(b"valor_vinte".to_vec()));
        assert_eq!(engine.get(30).unwrap(), None);

        // Pode continuar operando
        engine.put(30, b"valor_trinta_refeito".to_vec()).unwrap();
        assert_eq!(engine.get(30).unwrap(), Some(b"valor_trinta_refeito".to_vec()));
    }
}

#[test]
fn test_crash_recovery_corrupted_crc_at_eof_truncates_and_recovers() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // 1. Grava 2 registros válidos
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(100, b"alpha".to_vec()).unwrap();
        engine.put(200, b"beta".to_vec()).unwrap();
    }

    let intact_size = fs::metadata(&wal_path).unwrap().len();

    // 2. Anexa um registro completo mas com CRC corrompido no final do arquivo (típico de escrita corrompida no crash)
    {
        let record = LogRecord::put(300, b"gamma_corrompido".to_vec());
        let mut bytes = record.encode_to_vec();
        // Inverte o CRC
        bytes[0] ^= 0xFF;

        let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }

    // 3. Reabre o engine
    {
        let mut engine = StorageEngine::open(dir_path).expect("Falha ao recuperar de registro com CRC corrompido no EOF");
        let report = engine.recovery_report();

        assert!(report.truncated);
        assert_eq!(report.valid_records_count, 2);
        assert_eq!(report.valid_offset, intact_size);

        let repaired_size = fs::metadata(&wal_path).unwrap().len();
        assert_eq!(repaired_size, intact_size);

        assert_eq!(engine.get(100).unwrap(), Some(b"alpha".to_vec()));
        assert_eq!(engine.get(200).unwrap(), Some(b"beta".to_vec()));
        assert_eq!(engine.get(300).unwrap(), None);

        assert!(engine.verify().is_ok());
    }
}

#[test]
fn test_crash_recovery_single_trailing_byte_at_eof() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(1, b"unico".to_vec()).unwrap();
    }

    let _intact_size = fs::metadata(&wal_path).unwrap().len();

    // Anexa apenas 1 byte residual no fim do arquivo
    {
        let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
        file.write_all(&[0xAA]).unwrap();
        file.sync_all().unwrap();
    }

    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        let report = engine.recovery_report();
        assert!(report.truncated);
        assert_eq!(report.valid_records_count, 1);
        assert_eq!(report.truncated_bytes, 1);
        assert_eq!(engine.get(1).unwrap(), Some(b"unico".to_vec()));
    }
}

#[test]
fn test_crash_recovery_wal_with_only_corrupted_record() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // Cria o WAL diretamente com apenas 5 bytes inválidos
    fs::write(&wal_path, &[1, 2, 3, 4, 5]).unwrap();

    let mut engine = StorageEngine::open(dir_path).unwrap();
    assert_eq!(engine.len(), 0);
    assert!(engine.recovery_report().truncated);
    assert_eq!(engine.recovery_report().valid_records_count, 0);

    // O WAL deve ter sido truncado a zero bytes
    assert_eq!(fs::metadata(&wal_path).unwrap().len(), 0);

    // Pode realizar operações normalmente
    engine.put(1, b"sobreviveu".to_vec()).unwrap();
    assert_eq!(engine.get(1).unwrap(), Some(b"sobreviveu".to_vec()));
}

#[test]
fn test_corruption_in_middle_of_file_is_not_silently_truncated() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // 1. Grava 3 registros: R1, R2, R3
    let loc2;
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(1, b"registro_1".to_vec()).unwrap();
        loc2 = engine.put(2, b"registro_2".to_vec()).unwrap();
        engine.put(3, b"registro_3".to_vec()).unwrap();
    }

    // 2. Corrompe o segundo registro R2 no meio do arquivo (não é o último registro!)
    {
        let mut bytes = fs::read(&wal_path).unwrap();
        // loc2.offset é o início de R2. Vamos corromper o CRC de R2 (primeiro byte)
        bytes[loc2.offset as usize] ^= 0x55;
        fs::write(&wal_path, bytes).unwrap();
    }

    // 3. Tentar abrir o engine com corrupção no meio do arquivo DEVE FALHAR.
    // Não pode truncar silenciosamente R2 e R3, pois isso mascararia perda de dados no meio do log!
    let open_result = StorageEngine::open(dir_path);
    match open_result {
        Err(EngineError::CrcMismatch {
            offset, ..
        }) => {
            assert_eq!(offset, loc2.offset);
        }
        Err(other) => panic!("Esperado CrcMismatch no offset do meio, obtido: {other:?}"),
        Ok(_) => panic!("StorageEngine::open não deve ter sucesso quando há corrupção no meio do WAL"),
    }

    // 4. O comando CLI verify também deve acusar a corrupção física
    let verify_result = handle_verify(dir_path);
    assert!(verify_result.is_err(), "verify deve falhar diante de corrupção no meio do log");
}

#[test]
fn test_write_and_recover_after_crash_recovery_cycle() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let wal_path = dir_path.join(WAL_FILE_NAME);

    // 1. Grava dados
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(10, b"valor_original".to_vec()).unwrap();
    }

    // 2. Corrompe o final com bytes parciais
    {
        let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
        file.write_all(&[0xDE, 0xAD]).unwrap();
        file.sync_all().unwrap();
    }

    // 3. Recupera do crash, grava novos dados e deleta antigo
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert!(engine.recovery_report().truncated);

        engine.put(20, b"segundo_valor".to_vec()).unwrap();
        engine.delete(10).unwrap();
    }

    // 4. Nova reabertura limpa
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert!(!engine.recovery_report().truncated);
        assert_eq!(engine.get(10).unwrap(), None);
        assert_eq!(engine.get(20).unwrap(), Some(b"segundo_valor".to_vec()));

        let valid = engine.verify().unwrap();
        assert_eq!(valid, 3); // 1 PUT + 1 PUT + 1 DELETE
    }
}
