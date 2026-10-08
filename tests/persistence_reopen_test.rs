//! Suíte de Testes Automatizados da Etapa 1: Persistência e Reinicialização.
//!
//! Valida a sobrevivência dos dados gravados com PUT e exclusões com DELETE
//! após múltiplos encerramentos e reinicializações do StorageEngine a partir do disco.

use std::collections::HashMap;
use storage_engine::cli::handle_verify;
use storage_engine::engine::StorageEngine;
use tempfile::tempdir;

#[test]
fn test_persistence_basic_put_and_reopen() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    // 1. Primeira sessão: inserção de registros
    {
        let mut engine = StorageEngine::open(dir_path).expect("Falha ao abrir engine inicial");
        assert_eq!(engine.len(), 0);

        engine
            .put(101, b"primeiro_valor".to_vec())
            .expect("Falha no put 101");
        engine
            .put(202, b"segundo_valor".to_vec())
            .expect("Falha no put 202");
        engine
            .put(303, b"terceiro_valor".to_vec())
            .expect("Falha no put 303");

        assert_eq!(engine.len(), 3);
        assert_eq!(
            engine.get(101).unwrap(),
            Some(b"primeiro_valor".to_vec())
        );
        assert_eq!(
            engine.get(202).unwrap(),
            Some(b"segundo_valor".to_vec())
        );
        assert_eq!(
            engine.get(303).unwrap(),
            Some(b"terceiro_valor".to_vec())
        );
        // Engine fechado aqui via drop
    }

    // 2. Segunda sessão: reabertura do engine no mesmo diretório
    {
        let mut engine =
            StorageEngine::open(dir_path).expect("Falha ao reabrir engine após fechamento");

        assert_eq!(engine.len(), 3);
        assert_eq!(engine.recovery_report().valid_records_count, 3);
        assert!(!engine.recovery_report().truncated);

        // Verifica que todos os valores originais persistem intactos
        assert_eq!(
            engine.get(101).unwrap(),
            Some(b"primeiro_valor".to_vec())
        );
        assert_eq!(
            engine.get(202).unwrap(),
            Some(b"segundo_valor".to_vec())
        );
        assert_eq!(
            engine.get(303).unwrap(),
            Some(b"terceiro_valor".to_vec())
        );

        // Chave inexistente deve retornar None
        assert_eq!(engine.get(999).unwrap(), None);
    }
}

#[test]
fn test_persistence_overwrite_values_and_reopen() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    // Sessão 1: insere e depois sobrescreve
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(100, b"versao_1".to_vec()).unwrap();
        engine.put(100, b"versao_2".to_vec()).unwrap();
        engine.put(100, b"versao_final_3".to_vec()).unwrap();
        assert_eq!(engine.len(), 1);
    }

    // Sessão 2: reabre e confere se a versão mais recente é retornada
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 1);
        // No log WAL existem 3 registros gravados, mas apenas a última versão deve estar indexada
        assert_eq!(engine.recovery_report().valid_records_count, 3);
        assert_eq!(
            engine.get(100).unwrap(),
            Some(b"versao_final_3".to_vec())
        );
    }
}

#[test]
fn test_persistence_delete_tombstone_reopen_not_found() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    // Sessão 1: insere chaves e remove uma delas
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(10, b"chave_permanente".to_vec()).unwrap();
        engine.put(20, b"chave_a_ser_removida".to_vec()).unwrap();
        engine.put(30, b"outra_permanente".to_vec()).unwrap();

        let existed = engine.delete(20).unwrap();
        assert!(existed, "A chave 20 deveria existir antes de ser removida");
        assert_eq!(engine.len(), 2);
        assert_eq!(engine.get(20).unwrap(), None);
    }

    // Sessão 2: reabre e verifica se o tombstone é respeitado (não vaza chave removida)
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 2);
        assert_eq!(engine.recovery_report().valid_records_count, 4); // 3 PUTs + 1 DELETE

        assert_eq!(
            engine.get(10).unwrap(),
            Some(b"chave_permanente".to_vec())
        );
        assert_eq!(
            engine.get(30).unwrap(),
            Some(b"outra_permanente".to_vec())
        );

        // Chave 20 NÃO deve ser encontrada
        assert_eq!(engine.get(20).unwrap(), None);
        assert!(!engine.contains_key(&20));
    }
}

#[test]
fn test_persistence_reinsert_after_delete_and_reopen() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    // Sessão 1: Inserir -> Deletar -> Reinserir nova versão
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        engine.put(55, b"primeira_vida".to_vec()).unwrap();
        engine.delete(55).unwrap();
        assert_eq!(engine.get(55).unwrap(), None);

        engine.put(55, b"segunda_vida_renascida".to_vec()).unwrap();
        assert_eq!(
            engine.get(55).unwrap(),
            Some(b"segunda_vida_renascida".to_vec())
        );
    }

    // Sessão 2: Reabrir e certificar que a segunda vida persiste
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 1);
        assert_eq!(
            engine.get(55).unwrap(),
            Some(b"segunda_vida_renascida".to_vec())
        );
    }
}

#[test]
fn test_persistence_multiple_reopen_cycles_accumulating_records() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    // Ciclo 1: grava chaves 1..=10
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        for k in 1..=10 {
            engine
                .put(k, format!("valor_{k}_ciclo_1").into_bytes())
                .unwrap();
        }
        assert_eq!(engine.len(), 10);
    }

    // Ciclo 2: reabre, valida, deleta as pares (2, 4, 6, 8, 10) e insere 11..=15
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 10);

        for k in (2..=10).step_by(2) {
            assert!(engine.delete(k).unwrap());
        }

        for k in 11..=15 {
            engine
                .put(k, format!("valor_{k}_ciclo_2").into_bytes())
                .unwrap();
        }

        assert_eq!(engine.len(), 10); // 5 ímpares originais + 5 novas
    }

    // Ciclo 3: reabre, sobrescreve as ímpares originais com novo prefixo e insere mais 5
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 10);

        // Confere que pares continuam deletadas
        for k in (2..=10).step_by(2) {
            assert_eq!(engine.get(k).unwrap(), None);
        }

        // Sobrescreve ímpares
        for k in (1..=9).step_by(2) {
            engine
                .put(k, format!("sobrescrito_{k}_ciclo_3").into_bytes())
                .unwrap();
        }

        // Adiciona 16..=20
        for k in 16..=20 {
            engine
                .put(k, format!("valor_{k}_ciclo_3").into_bytes())
                .unwrap();
        }

        assert_eq!(engine.len(), 15);
    }

    // Ciclo 4: reabre e valida estado consolidado de todas as chaves
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 15);

        // 1..9 ímpares devem ter o valor sobrescrito
        for k in (1..=9).step_by(2) {
            let val = engine.get(k).unwrap().expect("chave impar deve existir");
            assert_eq!(val, format!("sobrescrito_{k}_ciclo_3").into_bytes());
        }

        // 2..10 pares devem estar ausentes
        for k in (2..=10).step_by(2) {
            assert_eq!(engine.get(k).unwrap(), None);
        }

        // 11..15 devem ter valor do ciclo 2
        for k in 11..=15 {
            let val = engine.get(k).unwrap().expect("chave 11..15 deve existir");
            assert_eq!(val, format!("valor_{k}_ciclo_2").into_bytes());
        }

        // 16..20 devem ter valor do ciclo 3
        for k in 16..=20 {
            let val = engine.get(k).unwrap().expect("chave 16..20 deve existir");
            assert_eq!(val, format!("valor_{k}_ciclo_3").into_bytes());
        }

        // Validação completa de integridade de todos os registros acumulados no WAL
        let valid_records = engine.verify().unwrap();
        // Contagem total de gravações no log:
        // Ciclo 1: 10 PUTs
        // Ciclo 2: 5 DELETEs + 5 PUTs = 10 registros
        // Ciclo 3: 5 PUTs (sobrescritas) + 5 PUTs (novas) = 10 registros
        // Total = 30 registros físicos no WAL
        assert_eq!(valid_records, 30);
    }
}

#[test]
fn test_persistence_large_dataset_and_verify() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();
    let total_keys = 500u64;

    let mut expected_map = HashMap::new();

    // 1. Inserir 500 registros com tamanhos e valores variados
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        for k in 1..=total_keys {
            let val = format!("payload_extenso_da_chave_{k:06}_dados_{}", "!".repeat((k % 120) as usize)).into_bytes();
            engine.put(k, val.clone()).unwrap();
            expected_map.insert(k, val);
        }

        // Deleta as chaves múltiplas de 5
        for k in (5..=total_keys).step_by(5) {
            engine.delete(k).unwrap();
            expected_map.remove(&k);
        }

        assert_eq!(engine.len(), expected_map.len());
    }

    // 2. Reabrir e checar todos os 500 registros individualmente
    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), expected_map.len());

        for k in 1..=total_keys {
            let res = engine.get(k).unwrap();
            if let Some(expected_val) = expected_map.get(&k) {
                assert_eq!(res.as_ref(), Some(expected_val), "Valor divergente na chave {k}");
            } else {
                assert_eq!(res, None, "Chave {k} deveria estar excluída");
            }
        }

        // Auditoria via verify do engine
        let verified_records = engine.verify().unwrap();
        assert_eq!(verified_records, (total_keys + total_keys / 5) as usize);

        // Auditoria via CLI handle_verify
        let cli_report = handle_verify(dir_path).unwrap();
        assert!(cli_report.is_valid);
        assert_eq!(cli_report.records_count, verified_records);
        assert!(cli_report.bytes_validated > 0);
    }
}

#[test]
fn test_persistence_empty_engine_reopen() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path();

    {
        let engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 0);
        assert!(engine.is_empty());
    }

    {
        let mut engine = StorageEngine::open(dir_path).unwrap();
        assert_eq!(engine.len(), 0);
        assert!(engine.is_empty());
        assert_eq!(engine.recovery_report().valid_records_count, 0);
        assert!(!engine.recovery_report().truncated);

        assert_eq!(engine.get(1).unwrap(), None);
        let valid_records = engine.verify().unwrap();
        assert_eq!(valid_records, 0);
    }
}
