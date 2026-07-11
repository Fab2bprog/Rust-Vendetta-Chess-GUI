//! Optional bridge between the `analysis` crate and the `db` crate.
//!
//! This module allows **persisting** a UCI [`AnalysisResult`] to `SQLite` and
//! **reloading** the best stored analysis for a position and an engine.
//!
//! It reuses existing functions from `db`:
//! - [`db::repository::position_repo::get_or_insert`] — FEN deduplication.
//! - [`db::repository::analysis_repo::insert`] / [`find_best`] — storage / reading.
//!
//! ## Example
//!
//! ```
//! use db::schema::open_in_memory;
//! use uci::engine::AnalysisResult;
//! use analysis::store::{store_analysis, load_best};
//!
//! const FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
//!
//! let conn   = open_in_memory().unwrap();
//! let result = AnalysisResult { best_move: "e2e4".into(), ponder: None, info_lines: vec![] };
//! let id     = store_analysis(&conn, FEN, "vendetta", &result).unwrap();
//! assert!(id > 0);
//!
//! let best = load_best(&conn, FEN, "vendetta").unwrap();
//! assert!(best.is_some());
//! assert_eq!(best.unwrap().best_move, "e2e4");
//! ```

use std::fmt;

use rusqlite::{Connection, OptionalExtension};

use db::repository::{analysis_repo, analysis_repo::NewAnalysis, position_repo};
use uci::{
    engine::AnalysisResult,
    parser::{UciInfo, UciScore},
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Re-exported so the caller doesn't have to import `db` separately.
pub use db::repository::analysis_repo::AnalysisRow;

/// Error returned by the `store` module's functions.
#[derive(Debug)]
pub enum StoreError {
    /// Underlying `SQLite` error.
    Db(rusqlite::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "erreur base de données : {e}"),
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extracts the best UCI line (multipv = 1, maximum depth).
fn extract_best_info(info_lines: &[UciInfo]) -> Option<&UciInfo> {
    info_lines
        .iter()
        .filter(|i| i.multipv.unwrap_or(1) == 1 && i.depth.is_some())
        .max_by_key(|i| i.depth.unwrap_or(0))
}

/// Converts a [`UciScore`] into `(score_cp, score_mate)` columns for the DB.
fn uci_score_to_db(score: Option<&UciScore>) -> (Option<i64>, Option<i64>) {
    match score {
        Some(UciScore::Centipawns(cp)) => (Some(i64::from(*cp)), None),
        Some(UciScore::Mate(m))        => (None, Some(i64::from(*m))),
        _                              => (None, None),
    }
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Persists an [`AnalysisResult`] to `SQLite` and returns the `id` of the inserted row.
///
/// The position is inserted (or looked up) via its FEN before the analysis.
/// Only the **principal line** (multipv = 1) at the maximum depth is
/// stored. If `info_lines` is empty, the analysis is still recorded
/// with `depth = 0` and no score.
///
/// # Errors
///
/// Returns [`StoreError::Db`] if a `SQLite` error occurs.
pub fn store_analysis(
    conn:         &Connection,
    position_fen: &str,
    engine_id:    &str,
    result:       &AnalysisResult,
) -> Result<i64, StoreError> {
    // 1. Position: get_or_insert (atomic).
    let position_id = position_repo::get_or_insert(conn, position_fen)?;

    // 2. Extract the best info line.
    let best = extract_best_info(&result.info_lines);

    let (depth, score_cp, score_mate, pv, nodes) = match best {
        Some(info) => {
            let (scp, sm) = uci_score_to_db(info.score.as_ref());
            let pv    = info.pv.join(" ");
            let nodes = info.nodes.map(|n| i64::try_from(n).unwrap_or(i64::MAX));
            (i64::from(info.depth.unwrap_or(0)), scp, sm, pv, nodes)
        }
        None => (0_i64, None, None, String::new(), None),
    };

    // 3. Insert the analysis.
    let new = NewAnalysis {
        position_id,
        engine:    engine_id,
        depth,
        score_cp,
        score_mate,
        best_move: &result.best_move,
        pv:        &pv,
        nodes,
        time_ms:   None,
        multipv:   1,
    };
    Ok(analysis_repo::insert(conn, &new)?)
}

/// Returns the best stored analysis for a FEN position and an engine.
///
/// "Best" means: maximum depth, multipv = 1 line.
/// Returns `None` if the position is unknown or no analysis exists.
///
/// # Errors
///
/// Returns [`StoreError::Db`] if a `SQLite` error occurs.
pub fn load_best(
    conn:         &Connection,
    position_fen: &str,
    engine_id:    &str,
) -> Result<Option<AnalysisRow>, StoreError> {
    // Look up the position's id without inserting it.
    let position_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM positions WHERE fen = ?1",
            [position_fen],
            |row| row.get(0),
        )
        .optional()?;

    let Some(pos_id) = position_id else {
        return Ok(None); // Unknown position → no analysis possible.
    };

    Ok(analysis_repo::find_best(conn, pos_id, engine_id)?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use db::schema::open_in_memory;
    use uci::{
        engine::AnalysisResult,
        parser::{UciInfo, UciScore},
    };

    const FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_result(best_move: &str, lines: Vec<UciInfo>) -> AnalysisResult {
        AnalysisResult { best_move: best_move.to_string(), ponder: None, info_lines: lines }
    }

    fn info_cp(depth: u32, cp: i32) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: Some(1),
            score:   Some(UciScore::Centipawns(cp)),
            pv:      vec!["e2e4".into(), "e7e5".into()],
            ..UciInfo::default()
        }
    }

    fn info_mate(depth: u32, mate_in: i32) -> UciInfo {
        UciInfo {
            depth:   Some(depth),
            multipv: Some(1),
            score:   Some(UciScore::Mate(mate_in)),
            pv:      vec!["d1h5".into()],
            ..UciInfo::default()
        }
    }

    // -----------------------------------------------------------------------
    // store_analysis
    // -----------------------------------------------------------------------

    #[test]
    fn test_store_returns_id() {
        let conn   = open_in_memory().unwrap();
        let result = make_result("e2e4", vec![info_cp(10, 30)]);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_store_position_auto_created() {
        // The position is created automatically if it doesn't exist.
        let conn   = open_in_memory().unwrap();
        let result = make_result("e2e4", vec![]);
        store_analysis(&conn, FEN, "engine", &result).unwrap();

        let pos_id: Option<i64> = conn
            .query_row("SELECT id FROM positions WHERE fen = ?1", [FEN], |r| r.get(0))
            .optional()
            .unwrap();
        assert!(pos_id.is_some());
    }

    #[test]
    fn test_store_best_move_stored() {
        let conn   = open_in_memory().unwrap();
        let result = make_result("d2d4", vec![info_cp(8, 15)]);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        let row    = analysis_repo::find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.best_move, "d2d4");
    }

    #[test]
    fn test_store_extracts_deepest_depth() {
        // Several depths → the largest is kept.
        let conn   = open_in_memory().unwrap();
        let lines  = vec![info_cp(5, 10), info_cp(12, 30), info_cp(8, 20)];
        let result = make_result("e2e4", lines);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        let row    = analysis_repo::find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.depth, 12);
        assert_eq!(row.score_cp, Some(30));
    }

    #[test]
    fn test_store_score_cp() {
        let conn   = open_in_memory().unwrap();
        let result = make_result("e2e4", vec![info_cp(10, 55)]);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        let row    = analysis_repo::find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.score_cp,   Some(55));
        assert_eq!(row.score_mate, None);
    }

    #[test]
    fn test_store_score_mate() {
        let conn   = open_in_memory().unwrap();
        let result = make_result("d1h5", vec![info_mate(7, 3)]);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        let row    = analysis_repo::find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.score_cp,   None);
        assert_eq!(row.score_mate, Some(3));
    }

    #[test]
    fn test_store_empty_info_lines() {
        // No info lines → depth=0, no score, best_move stored anyway.
        let conn   = open_in_memory().unwrap();
        let result = make_result("g1f3", vec![]);
        let id     = store_analysis(&conn, FEN, "engine", &result).unwrap();
        let row    = analysis_repo::find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.depth,    0);
        assert_eq!(row.best_move, "g1f3");
        assert_eq!(row.score_cp, None);
    }

    // -----------------------------------------------------------------------
    // load_best
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_best_unknown_fen() {
        let conn = open_in_memory().unwrap();
        let best = load_best(&conn, "unknown/fen w - - 0 1", "engine").unwrap();
        assert!(best.is_none());
    }

    #[test]
    fn test_load_best_after_store() {
        let conn   = open_in_memory().unwrap();
        let result = make_result("e2e4", vec![info_cp(15, 40)]);
        store_analysis(&conn, FEN, "engine", &result).unwrap();
        let best = load_best(&conn, FEN, "engine").unwrap().unwrap();
        assert_eq!(best.best_move, "e2e4");
        assert_eq!(best.depth, 15);
    }

    #[test]
    fn test_load_best_returns_none_if_no_analysis() {
        // Known position but no analysis for this engine.
        let conn = open_in_memory().unwrap();
        // Insert the position via a store with another engine.
        let result = make_result("e2e4", vec![info_cp(10, 20)]);
        store_analysis(&conn, FEN, "other_engine", &result).unwrap();
        let best = load_best(&conn, FEN, "engine").unwrap();
        assert!(best.is_none());
    }
}
