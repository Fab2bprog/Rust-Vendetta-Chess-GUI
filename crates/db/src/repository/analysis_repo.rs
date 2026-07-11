//! Repository for the `analyses` table.
//!
//! Each analysis is linked to a [`positions`](crate::repository::position_repo)
//! via `position_id`. Deleting a position cascades the deletion
//! of all its analyses (`ON DELETE CASCADE`).

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Row of the `analyses` table as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisRow {
    pub id:          i64,
    pub position_id: i64,
    pub engine:      String,
    pub depth:       i64,
    pub score_cp:    Option<i64>,
    pub score_mate:  Option<i64>,
    pub best_move:   String,
    pub pv:          String,
    pub nodes:       Option<i64>,
    pub time_ms:     Option<i64>,
    pub multipv:     i64,
    pub created_at:  String,
}

/// Data needed to insert a new analysis.
#[derive(Debug, Clone)]
pub struct NewAnalysis<'a> {
    pub position_id: i64,
    pub engine:      &'a str,
    pub depth:       i64,
    /// Score in centipawns. `None` if the score is in moves before mate.
    pub score_cp:    Option<i64>,
    /// Score in moves before mate. `None` if the score is in centipawns.
    pub score_mate:  Option<i64>,
    pub best_move:   &'a str,
    pub pv:          &'a str,
    pub nodes:       Option<i64>,
    pub time_ms:     Option<i64>,
    /// `MultiPV` line number (1 = best line).
    pub multipv:     i64,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Inserts a new analysis and returns its auto-incremented `id`.
///
/// # Errors
///
/// Returns an error if `position_id` is invalid (foreign key) or if
/// the database is locked.
pub fn insert(conn: &Connection, analysis: &NewAnalysis<'_>) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO analyses
             (position_id, engine, depth, score_cp, score_mate,
              best_move, pv, nodes, time_ms, multipv)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            analysis.position_id,
            analysis.engine,
            analysis.depth,
            analysis.score_cp,
            analysis.score_mate,
            analysis.best_move,
            analysis.pv,
            analysis.nodes,
            analysis.time_ms,
            analysis.multipv,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Looks up an analysis by its `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<AnalysisRow>> {
    conn.query_row(
        "SELECT id, position_id, engine, depth, score_cp, score_mate,
                best_move, pv, nodes, time_ms, multipv, created_at
         FROM analyses WHERE id = ?1",
        [id],
        row_to_analysis,
    )
    .optional()
}

/// Returns all analyses for a position, sorted by decreasing depth
/// then increasing `multipv`.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_position(conn: &Connection, position_id: i64) -> SqlResult<Vec<AnalysisRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, position_id, engine, depth, score_cp, score_mate,
                best_move, pv, nodes, time_ms, multipv, created_at
         FROM analyses
         WHERE position_id = ?1
         ORDER BY depth DESC, multipv ASC",
    )?;
    let rows = stmt.query_map([position_id], row_to_analysis)?;
    rows.collect()
}

/// Returns all analyses produced by a given engine,
/// sorted by increasing `id`.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_engine(conn: &Connection, engine: &str) -> SqlResult<Vec<AnalysisRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, position_id, engine, depth, score_cp, score_mate,
                best_move, pv, nodes, time_ms, multipv, created_at
         FROM analyses
         WHERE engine = ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([engine], row_to_analysis)?;
    rows.collect()
}

/// Returns the analysis at maximum depth for a given position and engine
/// (`MultiPV` line 1 only). Returns `None` if there is no analysis.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_best(
    conn: &Connection,
    position_id: i64,
    engine: &str,
) -> SqlResult<Option<AnalysisRow>> {
    conn.query_row(
        "SELECT id, position_id, engine, depth, score_cp, score_mate,
                best_move, pv, nodes, time_ms, multipv, created_at
         FROM analyses
         WHERE position_id = ?1 AND engine = ?2 AND multipv = 1
         ORDER BY depth DESC
         LIMIT 1",
        params![position_id, engine],
        row_to_analysis,
    )
    .optional()
}

/// Deletes an analysis by its `id`. Returns `true` if a row was deleted.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn delete(conn: &Connection, id: i64) -> SqlResult<bool> {
    let affected = conn.execute("DELETE FROM analyses WHERE id = ?1", [id])?;
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn row_to_analysis(row: &rusqlite::Row<'_>) -> rusqlite::Result<AnalysisRow> {
    Ok(AnalysisRow {
        id:          row.get(0)?,
        position_id: row.get(1)?,
        engine:      row.get(2)?,
        depth:       row.get(3)?,
        score_cp:    row.get(4)?,
        score_mate:  row.get(5)?,
        best_move:   row.get(6)?,
        pv:          row.get(7)?,
        nodes:       row.get(8)?,
        time_ms:     row.get(9)?,
        multipv:     row.get(10)?,
        created_at:  row.get(11)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{repository::position_repo, schema::open_in_memory};

    const FEN_START: &str =
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const FEN_E4: &str =
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";

    /// Creates a position in the database and returns its id.
    fn setup_position(conn: &Connection, fen: &str) -> i64 {
        position_repo::get_or_insert(conn, fen).unwrap()
    }

    fn minimal_analysis(position_id: i64) -> NewAnalysis<'static> {
        NewAnalysis {
            position_id,
            engine:     "vendetta_chess_motor",
            depth:      10,
            score_cp:   Some(42),
            score_mate: None,
            best_move:  "e2e4",
            pv:         "e2e4 e7e5 g1f3",
            nodes:      Some(100_000),
            time_ms:    Some(500),
            multipv:    1,
        }
    }

    #[test]
    fn test_insert_returns_id() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        let id = insert(&conn, &minimal_analysis(pos_id)).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_find_by_id_found() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        let id = insert(&conn, &minimal_analysis(pos_id)).unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.position_id, pos_id);
        assert_eq!(row.engine, "vendetta_chess_motor");
        assert_eq!(row.depth, 10);
        assert_eq!(row.score_cp, Some(42));
        assert_eq!(row.score_mate, None);
        assert_eq!(row.best_move, "e2e4");
        assert_eq!(row.multipv, 1);
    }

    #[test]
    fn test_find_by_id_not_found() {
        let conn = open_in_memory().unwrap();
        assert!(find_by_id(&conn, 9999).unwrap().is_none());
    }

    #[test]
    fn test_find_by_position_empty() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        assert!(find_by_position(&conn, pos_id).unwrap().is_empty());
    }

    #[test]
    fn test_find_by_position_multiple() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        // Depth 10, then 20
        insert(&conn, &minimal_analysis(pos_id)).unwrap();
        insert(
            &conn,
            &NewAnalysis {
                depth: 20,
                score_cp: Some(55),
                best_move: "d2d4",
                ..minimal_analysis(pos_id)
            },
        )
        .unwrap();
        let rows = find_by_position(&conn, pos_id).unwrap();
        assert_eq!(rows.len(), 2);
        // Sorted by depth DESC
        assert_eq!(rows[0].depth, 20);
        assert_eq!(rows[1].depth, 10);
    }

    #[test]
    fn test_find_by_position_only_own_position() {
        let conn = open_in_memory().unwrap();
        let pos1 = setup_position(&conn, FEN_START);
        let pos2 = setup_position(&conn, FEN_E4);
        insert(&conn, &minimal_analysis(pos1)).unwrap();
        insert(&conn, &minimal_analysis(pos2)).unwrap();
        let rows = find_by_position(&conn, pos1).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].position_id, pos1);
    }

    #[test]
    fn test_find_by_engine() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        insert(&conn, &minimal_analysis(pos_id)).unwrap();
        insert(
            &conn,
            &NewAnalysis {
                engine: "stockfish",
                ..minimal_analysis(pos_id)
            },
        )
        .unwrap();
        let rows = find_by_engine(&conn, "vendetta_chess_motor").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].engine, "vendetta_chess_motor");
    }

    #[test]
    fn test_find_by_engine_not_found() {
        let conn = open_in_memory().unwrap();
        let rows = find_by_engine(&conn, "unknown_engine").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_find_best_returns_deepest() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        insert(&conn, &minimal_analysis(pos_id)).unwrap(); // depth 10
        insert(
            &conn,
            &NewAnalysis { depth: 20, ..minimal_analysis(pos_id) },
        )
        .unwrap(); // depth 20
        insert(
            &conn,
            &NewAnalysis { depth: 15, ..minimal_analysis(pos_id) },
        )
        .unwrap(); // depth 15
        let best = find_best(&conn, pos_id, "vendetta_chess_motor")
            .unwrap()
            .unwrap();
        assert_eq!(best.depth, 20);
    }

    #[test]
    fn test_find_best_ignores_multipv_gt_1() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        // Only multipv=2 line, no multipv=1
        insert(
            &conn,
            &NewAnalysis {
                depth:  30,
                multipv: 2,
                ..minimal_analysis(pos_id)
            },
        )
        .unwrap();
        // find_best must not return this line
        let best = find_best(&conn, pos_id, "vendetta_chess_motor").unwrap();
        assert!(best.is_none());
    }

    #[test]
    fn test_find_best_none_when_empty() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        assert!(find_best(&conn, pos_id, "vendetta_chess_motor")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_score_mate_stored() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        let id = insert(
            &conn,
            &NewAnalysis {
                score_cp:   None,
                score_mate: Some(3),
                ..minimal_analysis(pos_id)
            },
        )
        .unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.score_cp, None);
        assert_eq!(row.score_mate, Some(3));
    }

    #[test]
    fn test_delete_existing() {
        let conn = open_in_memory().unwrap();
        let pos_id = setup_position(&conn, FEN_START);
        let id = insert(&conn, &minimal_analysis(pos_id)).unwrap();
        assert!(delete(&conn, id).unwrap());
        assert!(find_by_id(&conn, id).unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let conn = open_in_memory().unwrap();
        assert!(!delete(&conn, 9999).unwrap());
    }

    #[test]
    fn test_invalid_position_id_fails() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let result = insert(
            &conn,
            &NewAnalysis {
                position_id: 9999, // does not exist
                ..minimal_analysis(9999)
            },
        );
        assert!(result.is_err());
    }
}
