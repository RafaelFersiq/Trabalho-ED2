//! Suíte de Testes Automatizados da Etapa 1: Formato Binário e Validação de CRC32.
//!
//! Valida a integridade física de serialização e deserialização dos registros binários do WAL,
//! o cálculo com `crc32fast` e a detecção de corrupções em cabeçalho, payload e flags.

use std::io::Cursor;
use storage_engine::error::EngineError;
use storage_engine::wal::record::{calculate_crc, LogRecord, FLAG_DELETE, FLAG_PUT, HEADER_SIZE};

#[test]
fn test_binary_record_put_encoding_decoding_roundtrip() {
    let key = 1234567890123456789u64;
    let value = b"valor_persistente_ed2".to_vec();
    let record = LogRecord::put(key, value.clone());

    assert!(!record.is_tombstone);
    assert_eq!(record.key, key);
    assert_eq!(record.value, value);
    assert_eq!(record.encoded_size(), HEADER_SIZE + value.len());

    let mut buffer = Vec::new();
    let written_bytes = record
        .encode(&mut buffer)
        .expect("Falha ao serializar registro PUT");

    assert_eq!(written_bytes, record.encoded_size());
    assert_eq!(buffer.len(), written_bytes);

    // Validação da deserialização
    let mut cursor = Cursor::new(&buffer);
    let decoded = LogRecord::decode(&mut cursor, 0)
        .expect("Falha ao deserializar registro PUT")
        .expect("Esperado registro, mas EOF prematuro retornado");

    assert_eq!(decoded.key, key);
    assert_eq!(decoded.value, value);
    assert!(!decoded.is_tombstone);
}

#[test]
fn test_binary_record_delete_encoding_decoding_roundtrip() {
    let key = 9876543210987654321u64;
    let record = LogRecord::delete(key);

    assert!(record.is_tombstone);
    assert_eq!(record.key, key);
    assert!(record.value.is_empty());
    assert_eq!(record.encoded_size(), HEADER_SIZE);

    let mut buffer = Vec::new();
    let written_bytes = record
        .encode(&mut buffer)
        .expect("Falha ao serializar registro DELETE");

    assert_eq!(written_bytes, HEADER_SIZE);
    assert_eq!(buffer.len(), HEADER_SIZE);

    // Validação de flags no buffer
    assert_eq!(buffer[4], FLAG_DELETE);

    let mut cursor = Cursor::new(&buffer);
    let decoded = LogRecord::decode(&mut cursor, 128)
        .expect("Falha ao deserializar registro DELETE")
        .expect("Esperado registro, mas EOF prematuro retornado");

    assert_eq!(decoded.key, key);
    assert!(decoded.value.is_empty());
    assert!(decoded.is_tombstone);
}

#[test]
fn test_binary_record_empty_value_put() {
    let key = 42u64;
    let empty_value = Vec::new();
    let record = LogRecord::put(key, empty_value.clone());

    let mut buffer = Vec::new();
    let written_bytes = record
        .encode(&mut buffer)
        .expect("Falha ao serializar PUT com valor vazio");

    assert_eq!(written_bytes, HEADER_SIZE);

    let mut cursor = Cursor::new(&buffer);
    let decoded = LogRecord::decode(&mut cursor, 0)
        .expect("Falha ao deserializar PUT com valor vazio")
        .expect("Esperado registro");

    assert_eq!(decoded.key, key);
    assert_eq!(decoded.value, empty_value);
    assert!(!decoded.is_tombstone);
}

#[test]
fn test_binary_record_large_payload_and_binary_data() {
    let key = 0xDEADBEEFCAFEBABEu64;
    // Cria payload grande (64 KB) com bytes binários arbitrários incluindo zeros
    let mut large_value = Vec::with_capacity(65536);
    for i in 0..65536 {
        large_value.push((i % 256) as u8);
    }

    let record = LogRecord::put(key, large_value.clone());
    let mut buffer = Vec::new();
    let written_bytes = record.encode(&mut buffer).expect("Falha ao codificar");

    assert_eq!(written_bytes, HEADER_SIZE + 65536);

    let mut cursor = Cursor::new(&buffer);
    let decoded = LogRecord::decode(&mut cursor, 1024)
        .expect("Falha ao decodificar")
        .expect("Esperado registro íntegro");

    assert_eq!(decoded.key, key);
    assert_eq!(decoded.value, large_value);
}

#[test]
fn test_crc32_deterministic_calculation() {
    let flags = FLAG_PUT;
    let key = 1000u64;
    let val_len = 5u32;
    let value = b"teste";

    let crc1 = calculate_crc(flags, key, val_len, value);
    let crc2 = calculate_crc(flags, key, val_len, value);
    assert_eq!(crc1, crc2, "Cálculo de CRC32 deve ser determinístico");

    // Qualquer mudança nos parâmetros deve gerar CRC distinto
    let crc_different_flag = calculate_crc(FLAG_DELETE, key, val_len, value);
    assert_ne!(crc1, crc_different_flag);

    let crc_different_key = calculate_crc(flags, key + 1, val_len, value);
    assert_ne!(crc1, crc_different_key);

    let crc_different_val_len = calculate_crc(flags, key, val_len + 1, value);
    assert_ne!(crc1, crc_different_val_len);

    let crc_different_value = calculate_crc(flags, key, val_len, b"testf");
    assert_ne!(crc1, crc_different_value);
}

#[test]
fn test_crc32_detection_on_corrupted_flags() {
    let record = LogRecord::put(10, b"abc".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Offset 4 é a Flag
    buffer[4] ^= 0xFF;

    let mut cursor = Cursor::new(&buffer);
    let result = LogRecord::decode(&mut cursor, 0);
    assert!(result.is_err());
}

#[test]
fn test_crc32_detection_on_corrupted_key_bytes() {
    let record = LogRecord::put(500, b"teste_chave".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Bytes 5..13 são a Key (u64 little endian)
    buffer[5] ^= 0x01; // corrompe 1 bit da chave

    let mut cursor = Cursor::new(&buffer);
    match LogRecord::decode(&mut cursor, 50) {
        Err(EngineError::CrcMismatch {
            expected,
            calculated,
            offset,
        }) => {
            assert_ne!(expected, calculated);
            assert_eq!(offset, 50);
        }
        other => panic!("Esperado CrcMismatch, obtido: {other:?}"),
    }
}

#[test]
fn test_crc32_detection_on_corrupted_val_len_bytes() {
    let record = LogRecord::put(700, b"teste_tamanho".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Bytes 13..17 são ValLen (u32 little endian)
    buffer[13] ^= 0x01;

    let mut cursor = Cursor::new(&buffer);
    let result = LogRecord::decode(&mut cursor, 70);
    assert!(result.is_err(), "Deve falhar ao desserializar com ValLen corrompido");
}

#[test]
fn test_crc32_detection_on_corrupted_value_bytes() {
    let record = LogRecord::put(999, b"conteudo_seguro".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Bytes a partir de HEADER_SIZE (17) são o value
    buffer[HEADER_SIZE] ^= 0x01; // corrompe 1 bit do valor

    let mut cursor = Cursor::new(&buffer);
    match LogRecord::decode(&mut cursor, 200) {
        Err(EngineError::CrcMismatch {
            expected,
            calculated,
            offset,
        }) => {
            assert_ne!(expected, calculated);
            assert_eq!(offset, 200);
        }
        other => panic!("Esperado CrcMismatch, obtido: {other:?}"),
    }
}

#[test]
fn test_crc32_detection_on_corrupted_crc_header_field() {
    let record = LogRecord::put(888, b"valor_qualquer".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Bytes 0..4 são o próprio CRC gravado
    buffer[0] ^= 0x80;

    let mut cursor = Cursor::new(&buffer);
    match LogRecord::decode(&mut cursor, 350) {
        Err(EngineError::CrcMismatch {
            expected,
            calculated,
            offset,
        }) => {
            assert_ne!(expected, calculated);
            assert_eq!(offset, 350);
        }
        other => panic!("Esperado CrcMismatch, obtido: {other:?}"),
    }
}

#[test]
fn test_decode_clean_eof() {
    let buffer = Vec::new();
    let mut cursor = Cursor::new(&buffer);
    let result = LogRecord::decode(&mut cursor, 0).expect("EOF limpo não deve ser erro");
    assert!(result.is_none(), "EOF limpo deve retornar Ok(None)");
}

#[test]
fn test_decode_truncated_header_unexpected_eof() {
    let record = LogRecord::put(123, b"dados".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Trunca no meio do cabeçalho (apenas 10 bytes dos 17 de HEADER_SIZE)
    let truncated_buffer = &buffer[..10];
    let mut cursor = Cursor::new(truncated_buffer);

    match LogRecord::decode(&mut cursor, 0) {
        Err(EngineError::UnexpectedEof) => {}
        other => panic!("Esperado UnexpectedEof, obtido: {other:?}"),
    }
}

#[test]
fn test_decode_truncated_value_unexpected_eof() {
    let record = LogRecord::put(123, b"dados_muito_longos".to_vec());
    let mut buffer = Vec::new();
    record.encode(&mut buffer).unwrap();

    // Trunca o payload de valor na metade
    let truncated_buffer = &buffer[..HEADER_SIZE + 4];
    let mut cursor = Cursor::new(truncated_buffer);

    match LogRecord::decode(&mut cursor, 0) {
        Err(EngineError::UnexpectedEof) => {}
        other => panic!("Esperado UnexpectedEof, obtido: {other:?}"),
    }
}

#[test]
fn test_decode_invalid_flag() {
    let mut buffer = vec![0u8; HEADER_SIZE];
    // Coloca flag inválida 0x99 no byte 4
    buffer[4] = 0x99;

    let mut cursor = Cursor::new(&buffer);
    match LogRecord::decode(&mut cursor, 0) {
        Err(EngineError::InvalidRecord(msg)) => {
            assert!(msg.contains("0x99") || msg.contains("153"));
        }
        other => panic!("Esperado InvalidRecord com flag, obtido: {other:?}"),
    }
}

#[test]
fn test_decode_delete_record_with_nonzero_val_len() {
    // Monta registro com flag DELETE mas val_len = 10
    let mut buffer = Vec::new();
    let flags = FLAG_DELETE;
    let key = 100u64;
    let val_len = 10u32;
    let payload = vec![0u8; 10];
    let crc = calculate_crc(flags, key, val_len, &payload);

    buffer.extend_from_slice(&crc.to_le_bytes());
    buffer.push(flags);
    buffer.extend_from_slice(&key.to_le_bytes());
    buffer.extend_from_slice(&val_len.to_le_bytes());
    buffer.extend_from_slice(&payload);

    let mut cursor = Cursor::new(&buffer);
    match LogRecord::decode(&mut cursor, 0) {
        Err(EngineError::InvalidRecord(msg)) => {
            assert!(msg.contains("tombstone") || msg.contains("exclusão"));
        }
        other => panic!("Esperado InvalidRecord com aviso de tombstone, obtido: {other:?}"),
    }
}

