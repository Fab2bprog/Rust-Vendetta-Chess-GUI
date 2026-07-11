//! Repository for the `positions` table.
//!
//! Positions are **deduplicated by FEN**: the same FEN is stored
//! only once. [`get_or_insert`] is the central operation — it
//! returns the existing `id` or inserts the row if absent.

use rusqlite::{Connection, OptionalExtension, Result as SqlResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Row of the `positions` table as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionRow {
    pub id:         i64,
    pub fen:        String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Returns the `id` of the position matching `fen`, inserting it if
/// it doesn't already exist.
///
/// This operation is atomic thanks to the `UNIQUE` constraint on `fen`
/// and the `INSERT OR IGNORE` clause.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn get_or_insert(conn: &Connection, fen: &str) -> SqlResult<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO positions (fen) VALUES (?1)",
        [fen],
    )?;
    conn.query_row(
        "SELECT id FROM positions WHERE fen = ?1",
        [fen],
        |row| row.get(0),
    )
}

/// Looks up a position by its `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<PositionRow>> {
    conn.query_row(
        "SELECT id, fen, created_at FROM positions WHERE id = ?1",
        [id],
        row_to_position,
    )
    .optional()
}

/// Looks up a position by its exact FEN. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_fen(conn: &Connection, fen: &str) -> SqlResult<Option<PositionRow>> {
    conn.query_row(
        "SELECT id, fen, created_at FROM positions WHERE fen = ?1",
        [fen],
        row_to_position,
    )
    .optional()
}

/// Returns all positions, sorted by increasing `id`.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_all(conn: &Connection) -> SqlResult<Vec<PositionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, fen, created_at FROM positions ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], row_to_position)?;
    rows.collect()
}

/// Deletes a position by its `id`. Returns `true` if a row was deleted.
///
/// Linked analyses are deleted in cascade (`ON DELETE CASCADE`).
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn delete(conn: &Connection, id: i64) -> SqlResult<bool> {
    let affected = conn.execute("DELETE FROM positions WHERE id = ?1", [id])?;
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn row_to_position(row: &rusqlite::Row<'_>) -> rusqlite::Result<PositionRow> {
    Ok(PositionRow {
        id:         row.get(0)?,
        fen:        row.get(1)?,
        created_at: row.get(2)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::open_in_memory;

    const FEN_START: &str =
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const FEN_E4: &str =
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";

    #[test]
    fn test_get_or_insert_new() {
        let conn = open_in_memory().unwrap();
        let id = get_or_insert(&conn, FEN_START).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_get_or_insert_idempotent() {
        let conn = open_in_memory().unwrap();
        let id1 = get_or_insert(&conn, FEN_START).unwrap();
        let id2 = get_or_insert(&conn, FEN_START).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_get_or_insert_different_fens() {
        let conn = open_in_memory().unwrap();
        let id1 = get_or_insert(&conn, FEN_START).unwrap();
        let id2 = get_or_insert(&conn, FEN_E4).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_find_by_id_found() {
        let conn = open_in_memory().unwrap();
        let id = get_or_insert(&conn, FEN_START).unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.fen, FEN_START);
    }

    #[test]
    fn test_find_by_id_not_found() {
        let conn = open_in_memory().unwrap();
        assert!(find_by_id(&conn, 9999).unwrap().is_none());
    }

    #[test]
    fn test_find_by_fen_found() {
        let conn = open_in_memory().unwrap();
        let id = get_or_insert(&conn, FEN_START).unwrap();
        let row = find_by_fen(&conn, FEN_START).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.fen, FEN_START);
    }

    #[test]
    fn test_find_by_fen_not_found() {
        let conn = open_in_memory().unwrap();
        assert!(find_by_fen(&conn, FEN_E4).unwrap().is_none());
    }

    #[test]
    fn test_find_all_empty() {
        let conn = open_in_memory().unwrap();
        assert!(find_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_find_all_multiple() {
        let conn = open_in_memory().unwrap();
        get_or_insert(&conn, FEN_START).unwrap();
        get_or_insert(&conn, FEN_E4).unwrap();
        let rows = find_all(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fen, FEN_START);
        assert_eq!(rows[1].fen, FEN_E4);
    }

    #[test]
    fn test_delete_existing() {
        let conn = open_in_memory().unwrap();
        let id = get_or_insert(&conn, FEN_START).unwrap();
        assert!(delete(&conn, id).unwrap());
        assert!(find_by_id(&conn, id).unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let conn = open_in_memory().unwrap();
        assert!(!delete(&conn, 9999).unwrap());
    }

    #[test]
    fn test_delete_cascades_to_analyses() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let pos_id = get_or_insert(&conn, FEN_START).unwrap();
        conn.execute(
            "INSERT INTO analyses (position_id, engine, depth, best_move)
             VALUES (?1, 'test_engine', 10, 'e2e4')",
            [pos_id],
        )
        .unwrap();
        // Deleting the position → analyses deleted in cascade
        delete(&conn, pos_id).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM analyses WHERE position_id = ?1",
                [pos_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_or_insert_multiple_calls_only_one_row() {
        let conn = open_in_memory().unwrap();
        for _ in 0..5 {
            get_or_insert(&conn, FEN_START).unwrap();
        }
        let rows = find_all(&conn).unwrap();
        assert_eq!(rows.len(), 1);
    }
}
