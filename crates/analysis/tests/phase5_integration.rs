//! Integration test — Phase 5: full validation of the Analysis pipeline.
//!
//! Scenario:
//! 1. `vendetta_chess_motor` analyzes a real position.
//! 2. The result is aggregated, turned into a graph series, and stored in `SQLite`.
//! 3. We reload the best analysis and check consistency.
//! 4. Two engines analyze the same position; their scores are compared.
//!
//! These tests are silently skipped if `vendetta_chess_motor` is absent.

use std::{path::PathBuf, time::Duration};

use analysis::{
    aggregator::aggregate,
    comparator::{compare_engines, EngineResult},
    graph::{build_series, score_range},
    store::{load_best, store_analysis},
};
use db::schema::open_in_memory;
use engine::{comparator::EvalComparator, config::EngineConfig};
use uci::{
    engine::{EnginePosition, UciEngine},
    protocol::GoLimits,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

const FEN_START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

// ---------------------------------------------------------------------------
// Test 1: full pipeline analyze → aggregate → graph → store → load
// ---------------------------------------------------------------------------

/// Full Phase 5 pipeline: analysis → aggregation → graph → `SQLite` ↔ reload.
#[test]
fn test_phase5_analyze_aggregate_store_load() {
    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    // --- 1. Analyze ---
    let path_str   = vendetta_path().to_str().unwrap().to_owned();
    let mut engine = UciEngine::connect_with_timeout(&path_str, Duration::from_secs(10)).unwrap();
    let position   = EnginePosition::start();
    let limits     = GoLimits { movetime: Some(300), ..GoLimits::default() };
    let result     = engine.analyze(&position, &limits).unwrap();
    engine.quit();

    assert!(!result.best_move.is_empty(), "bestmove ne doit pas être vide");
    assert!(!result.info_lines.is_empty(), "des lignes info sont attendues");

    // --- 2. Aggregate ---
    let agg = aggregate(&result);
    let pl  = agg.principal_line().expect("ligne principale attendue");
    assert!(pl.best_depth > 0, "profondeur doit être > 0");
    assert_eq!(
        agg.best_move, result.best_move,
        "best_move agrégé doit correspondre au bestmove UCI"
    );

    eprintln!(
        "Agrégation OK : best_move={}, depth={}, score_cp={:?}",
        agg.best_move, pl.best_depth, agg.score_cp()
    );

    // --- 3. Graph series ---
    let series = build_series("vendetta", &result.info_lines);
    assert!(!series.is_empty(), "la série ne doit pas être vide");
    assert!(
        series.max_depth().unwrap_or(0) > 0,
        "profondeur max doit être > 0"
    );
    assert!(
        score_range(std::slice::from_ref(&series)).is_some(),
        "score_range doit retourner Some"
    );

    // --- 4. Store in SQLite ---
    let conn = open_in_memory().unwrap();
    let id   = store_analysis(&conn, FEN_START, "vendetta", &result).unwrap();
    assert!(id > 0, "id de l'analyse insérée doit être > 0");

    // --- 5. Reload and check ---
    let best = load_best(&conn, FEN_START, "vendetta").unwrap().unwrap();
    assert_eq!(
        best.best_move, result.best_move,
        "best_move rechargé doit correspondre"
    );
    assert!(best.depth > 0, "profondeur rechargée doit être > 0");

    eprintln!(
        "Store/Load OK : id={id}, best_move={}, depth={}",
        best.best_move, best.depth
    );
}

// ---------------------------------------------------------------------------
// Test 2: graph series consistency
// ---------------------------------------------------------------------------

/// Checks that the graph series is sorted and that the final score is present.
#[test]
fn test_phase5_graph_series_coherence() {
    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    let path_str   = vendetta_path().to_str().unwrap().to_owned();
    let mut engine = UciEngine::connect_with_timeout(&path_str, Duration::from_secs(10)).unwrap();
    let position   = EnginePosition::start();
    let limits     = GoLimits { movetime: Some(400), ..GoLimits::default() };
    let result     = engine.analyze(&position, &limits).unwrap();
    engine.quit();

    let series = build_series("vendetta", &result.info_lines);
    assert!(!series.is_empty(), "série non vide attendue");

    // Points sorted by increasing depth.
    let depths: Vec<u32> = series.points.iter().map(|p| p.depth).collect();
    let mut sorted = depths.clone();
    sorted.sort_unstable();
    assert_eq!(
        depths, sorted,
        "les points doivent être triés par profondeur croissante"
    );

    // The final score is available.
    assert!(
        series.latest_score_cp().is_some(),
        "score final attendu"
    );

    eprintln!(
        "Graph OK : {} points, profondeur max={:?}, score final={:?}",
        series.len(),
        series.max_depth(),
        series.latest_score_cp()
    );
}

// ---------------------------------------------------------------------------
// Test 3: compare two engines → ranking
// ---------------------------------------------------------------------------

/// Two engines analyze the same position; the ranking must be consistent.
#[test]
fn test_phase5_compare_engines_and_rank() {
    if !vendetta_path().exists() {
        eprintln!("vendetta_chess_motor absent — test ignoré");
        return;
    }

    // Analyze with 2 engines in parallel.
    let mut comparator = EvalComparator::new();
    comparator.add("va", &vendetta_config("VA")).unwrap();
    comparator.add("vb", &vendetta_config("VB")).unwrap();

    let position = EnginePosition::start();
    let limits   = GoLimits { movetime: Some(300), ..GoLimits::default() };
    let results  = comparator.compare(&position, &limits);

    assert_eq!(results.len(), 2, "deux résultats attendus");
    assert!(
        results.iter().all(engine::comparator::CompareResult::is_ok),
        "tous les moteurs doivent réussir"
    );

    // Aggregate and create EngineResult entries.
    let engine_results: Vec<EngineResult> = results
        .iter()
        .filter_map(|cr| {
            cr.result.as_ref().ok().map(|ar| {
                EngineResult::new(cr.engine_id.clone(), aggregate(ar))
            })
        })
        .collect();

    assert_eq!(engine_results.len(), 2, "deux EngineResult attendus");

    // Compare and check the ranking.
    let ranked = compare_engines(&engine_results);
    assert_eq!(ranked.len(), 2, "deux rangs attendus");
    assert_eq!(ranked[0].rank, 1, "rang 1 attendu en premier");
    assert_eq!(ranked[1].rank, 2, "rang 2 attendu en second");

    // Each best_move must be a valid UCI move (4 characters).
    for r in &ranked {
        assert_eq!(
            r.best_move.len(), 4,
            "best_move UCI invalide pour '{}' : '{}'",
            r.engine_id, r.best_move
        );
    }

    eprintln!(
        "Classement OK :\n  #1 {} → {} (score_cp={:?})\n  #2 {} → {} (score_cp={:?})",
        ranked[0].engine_id, ranked[0].best_move, ranked[0].score_cp,
        ranked[1].engine_id, ranked[1].best_move, ranked[1].score_cp,
    );
}
