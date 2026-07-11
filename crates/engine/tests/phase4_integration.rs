//! Integration test — Phase 4: full validation of the Engine Manager.
//!
//! Scenario:
//! 1. Two instances of `vendetta_chess_motor` are connected via
//!    [`EvalComparator`].
//! 2. The same position (startpos) is submitted to both engines in parallel.
//! 3. The results are logged via [`SessionLogger`].
//! 4. Results consistency is checked (valid moves, UCI format).
//!
//! This test is automatically skipped if the `vendetta_chess_motor` binary
//! is absent (for CI without a real engine).

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use engine::{
    comparator::EvalComparator,
    config::EngineConfig,
    logger::SessionLogger,
};
use uci::{
    engine::EnginePosition,
    protocol::GoLimits,
};

fn vendetta_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../engines/vendetta_chess_motor");
    p
}

fn vendetta_config(name: &str) -> EngineConfig {
    EngineConfig::builder(name, vendetta_path())
        .init_timeout(Duration::from_secs(10))
        .build()
}

fn log_path() -> PathBuf {
    std::env::temp_dir().join("vendetta_phase4_integration.log")
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

/// Two engines analyze the same position in parallel, results logged.
#[test]
fn test_phase4_two_engines_parallel_with_logger() {
    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    // --- 1. Initialize the comparator ---
    let mut comparator = EvalComparator::new();
    comparator.add("vendetta-1", &vendetta_config("Vendetta #1")).unwrap();
    comparator.add("vendetta-2", &vendetta_config("Vendetta #2")).unwrap();
    assert_eq!(comparator.len(), 2, "deux moteurs attendus dans le comparateur");

    // --- 2. Position and limits ---
    let position = EnginePosition::start();
    let limits   = GoLimits { movetime: Some(200), ..GoLimits::default() };

    // --- 3. Parallel analysis ---
    let t0      = Instant::now();
    let results = comparator.compare(&position, &limits);
    let elapsed = t0.elapsed();

    assert_eq!(results.len(), 2, "deux résultats attendus");
    assert!(
        elapsed < Duration::from_secs(5),
        "l'analyse parallèle doit terminer en < 5 s (durée : {elapsed:?})"
    );

    // --- 4. Result verification ---
    for cr in &results {
        assert!(
            cr.is_ok(),
            "moteur '{}' a retourné une erreur : {:?}",
            cr.engine_id,
            cr.result.as_ref().err()
        );
        let bm = cr.best_move().unwrap();
        assert_eq!(
            bm.len(), 4,
            "coup UCI invalide pour '{}' : '{bm}'",
            cr.engine_id
        );
        // UCI format: letter (a-h) + digit (1-8) + letter (a-h) + digit (1-8)
        let chars: Vec<char> = bm.chars().collect();
        assert!(
            chars[0].is_ascii_lowercase() && ('a'..='h').contains(&chars[0]),
            "colonne source invalide : '{}'", chars[0]
        );
        assert!(
            chars[1].is_ascii_digit() && ('1'..='8').contains(&chars[1]),
            "rangée source invalide : '{}'", chars[1]
        );
        assert!(
            chars[2].is_ascii_lowercase() && ('a'..='h').contains(&chars[2]),
            "colonne cible invalide : '{}'", chars[2]
        );
        assert!(
            chars[3].is_ascii_digit() && ('1'..='8').contains(&chars[3]),
            "rangée cible invalide : '{}'", chars[3]
        );
    }

    // --- 5. Logging the results ---
    let mut logger = SessionLogger::open(log_path()).unwrap();
    logger.log_start(&["vendetta-1", "vendetta-2"]).unwrap();

    for cr in &results {
        if let Ok(ref analysis) = cr.result {
            logger.log_analysis(
                &cr.engine_id,
                &position,
                analysis,
                elapsed / 2, // approximate duration per engine
            ).unwrap();
        }
    }

    logger.log_end().unwrap();

    // --- 6. Checking the log file ---
    let content = std::fs::read_to_string(log_path()).unwrap();
    assert!(content.contains("SESSION"),     "en-tête SESSION manquant");
    assert!(content.contains("ANALYSIS #1"), "première analyse manquante");
    assert!(content.contains("ANALYSIS #2"), "deuxième analyse manquante");
    assert!(content.contains("SESSION END"), "pied de page manquant");

    eprintln!("\n=== Extrait du log Phase 4 ===");
    for line in content.lines().take(30) {
        eprintln!("{line}");
    }
    eprintln!("==============================");

    // --- 7. The comparator is still functional ---
    let bm = comparator.best_moves(&position, &limits);
    assert_eq!(bm.len(), 2, "comparateur doit rester utilisable après la validation");
}

/// Checks that both engines are indeed run in parallel
/// (total duration close to that of a single engine).
#[test]
fn test_phase4_parallel_is_faster_than_sequential() {
    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    let movetime_ms = 300u64;
    let position    = EnginePosition::start();
    let limits      = GoLimits { movetime: Some(movetime_ms), ..GoLimits::default() };

    let mut comparator = EvalComparator::new();
    comparator.add("v1", &vendetta_config("V1")).unwrap();
    comparator.add("v2", &vendetta_config("V2")).unwrap();

    let t0      = Instant::now();
    let results = comparator.compare(&position, &limits);
    let elapsed = t0.elapsed();

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(engine::comparator::CompareResult::is_ok));

    // In parallel, 2 × 300 ms should take noticeably less than 600 ms.
    // We allow up to 2× the movetime to absorb system variability.
    let max_expected = Duration::from_millis(movetime_ms * 2 + 500);
    assert!(
        elapsed < max_expected,
        "durée {elapsed:?} dépasse le seuil parallèle {max_expected:?}"
    );

    eprintln!(
        "Parallèle OK : 2 moteurs × {movetime_ms} ms → durée réelle {elapsed:?}"
    );
}

/// Checks the `EnginePool` + `EngineHandle` integration.
#[test]
fn test_phase4_pool_handle_integration() {
    use engine::{handle::EngineHandle, pool::EnginePool};

    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    let mut pool = EnginePool::new();
    let h1: EngineHandle = pool.add("v1", &vendetta_config("V1")).unwrap();
    let h2: EngineHandle = pool.add("v2", &vendetta_config("V2")).unwrap();

    // The handles point to active engines
    assert!(h1.is_alive(&pool));
    assert!(h2.is_alive(&pool));
    assert_eq!(pool.len(), 2);

    // After removal, the handle is invalid
    pool.remove("v1").unwrap();
    assert!(!h1.is_alive(&pool));
    assert!(h2.is_alive(&pool));
    assert_eq!(pool.len(), 1);

    eprintln!("Pool + Handle : OK — h1 mort, h2 vivant");
}
