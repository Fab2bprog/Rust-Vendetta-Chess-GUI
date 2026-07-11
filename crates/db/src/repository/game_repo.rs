//! Repository for the `games` table.
//!
//! All functions receive a `&Connection`; transaction management
//! is left to the caller, to keep the flexibility of a
//! multi-table transaction.

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Row of the `games` table as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct GameRow {
    pub id:            i64,
    pub tournament_id: Option<i64>,
    pub white:         String,
    pub black:         String,
    pub result:        String,
    pub date:          Option<String>,
    pub event:         Option<String>,
    pub site:          Option<String>,
    pub round:         Option<String>,
    pub pgn:           String,
    pub initial_fen:   String,
    pub move_count:    i64,
    pub created_at:    String,
}

/// Data needed to insert a new game.
#[derive(Debug, Clone)]
pub struct NewGame<'a> {
    pub tournament_id: Option<i64>,
    pub white:         &'a str,
    pub black:         &'a str,
    pub result:        &'a str,
    pub date:          Option<&'a str>,
    pub event:         Option<&'a str>,
    pub site:          Option<&'a str>,
    pub round:         Option<&'a str>,
    pub pgn:           &'a str,
    pub initial_fen:   Option<&'a str>,
    pub move_count:    i64,
}

// Default starting FEN (used if `initial_fen` is `None`).
const DEFAULT_FEN: &str =
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Inserts a new game and returns its auto-incremented `id`.
///
/// # Errors
///
/// Returns a `SQLite` error if the foreign key constraint is violated
/// (e.g. nonexistent `tournament_id`) or if the database is locked.
pub fn insert(conn: &Connection, game: &NewGame<'_>) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO games
             (tournament_id, white, black, result, date, event, site, round,
              pgn, initial_fen, move_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            game.tournament_id,
            game.white,
            game.black,
            game.result,
            game.date,
            game.event,
            game.site,
            game.round,
            game.pgn,
            game.initial_fen.unwrap_or(DEFAULT_FEN),
            game.move_count,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Looks up a game by its `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<GameRow>> {
    conn.query_row(
        "SELECT id, tournament_id, white, black, result, date, event, site,
                round, pgn, initial_fen, move_count, created_at
         FROM games WHERE id = ?1",
        [id],
        row_to_game,
    )
    .optional()
}

/// Returns all games, sorted by increasing `id`.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_all(conn: &Connection) -> SqlResult<Vec<GameRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, tournament_id, white, black, result, date, event, site,
                round, pgn, initial_fen, move_count, created_at
         FROM games ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], row_to_game)?;
    rows.collect()
}

/// Returns all games where `name` plays White or Black.
///
/// The comparison is case-insensitive.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_player(conn: &Connection, name: &str) -> SqlResult<Vec<GameRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, tournament_id, white, black, result, date, event, site,
                round, pgn, initial_fen, move_count, created_at
         FROM games
         WHERE lower(white) = lower(?1) OR lower(black) = lower(?1)
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([name], row_to_game)?;
    rows.collect()
}

/// Deletes a game by its `id`. Returns `true` if a row was deleted.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn delete(conn: &Connection, id: i64) -> SqlResult<bool> {
    let affected = conn.execute("DELETE FROM games WHERE id = ?1", [id])?;
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Maps a `SQLite` row to a [`GameRow`].
fn row_to_game(row: &rusqlite::Row<'_>) -> rusqlite::Result<GameRow> {
    Ok(GameRow {
        id:            row.get(0)?,
        tournament_id: row.get(1)?,
        white:         row.get(2)?,
        black:         row.get(3)?,
        result:        row.get(4)?,
        date:          row.get(5)?,
        event:         row.get(6)?,
        site:          row.get(7)?,
        round:         row.get(8)?,
        pgn:           row.get(9)?,
        initial_fen:   row.get(10)?,
        move_count:    row.get(11)?,
        created_at:    row.get(12)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::open_in_memory;

    fn minimal_game<'a>() -> NewGame<'a> {
        NewGame {
            tournament_id: None,
            white:         "Alice",
            black:         "Bob",
            result:        "1-0",
            date:          None,
            event:         None,
            site:          None,
            round:         None,
            pgn:           "[Event \"Test\"]\n\n1. e4 1-0",
            initial_fen:   None,
            move_count:    1,
        }
    }

    #[test]
    fn test_insert_returns_id() {
        let conn = open_in_memory().unwrap();
        let id = insert(&conn, &minimal_game()).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_find_by_id_found() {
        let conn = open_in_memory().unwrap();
        let id = insert(&conn, &minimal_game()).unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.white, "Alice");
        assert_eq!(row.black, "Bob");
        assert_eq!(row.result, "1-0");
        assert_eq!(row.move_count, 1);
    }

    #[test]
    fn test_find_by_id_not_found() {
        let conn = open_in_memory().unwrap();
        let row = find_by_id(&conn, 9999).unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn test_find_all_empty() {
        let conn = open_in_memory().unwrap();
        let rows = find_all(&conn).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_find_all_multiple() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &minimal_game()).unwrap();
        insert(
            &conn,
            &NewGame {
                white: "Carol",
                black: "Dave",
                result: "0-1",
                pgn: "",
                ..minimal_game()
            },
        )
        .unwrap();
        let rows = find_all(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].white, "Alice");
        assert_eq!(rows[1].white, "Carol");
    }

    #[test]
    fn test_find_by_player_white() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &minimal_game()).unwrap();
        let rows = find_by_player(&conn, "Alice").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white, "Alice");
    }

    #[test]
    fn test_find_by_player_black() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &minimal_game()).unwrap();
        let rows = find_by_player(&conn, "Bob").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].black, "Bob");
    }

    #[test]
    fn test_find_by_player_case_insensitive() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &minimal_game()).unwrap();
        let rows = find_by_player(&conn, "ALICE").unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn test_find_by_player_not_found() {
        let conn = open_in_memory().unwrap();
        insert(&conn, &minimal_game()).unwrap();
        let rows = find_by_player(&conn, "Unknown").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_delete_existing() {
        let conn = open_in_memory().unwrap();
        let id = insert(&conn, &minimal_game()).unwrap();
        let deleted = delete(&conn, id).unwrap();
        assert!(deleted);
        assert!(find_by_id(&conn, id).unwrap().is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let conn = open_in_memory().unwrap();
        let deleted = delete(&conn, 9999).unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_default_fen_applied() {
        let conn = open_in_memory().unwrap();
        let id = insert(&conn, &minimal_game()).unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.initial_fen, DEFAULT_FEN);
    }

    #[test]
    fn test_custom_fen_stored() {
        let conn = open_in_memory().unwrap();
        let custom_fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
        let id = insert(
            &conn,
            &NewGame {
                initial_fen: Some(custom_fen),
                ..minimal_game()
            },
        )
        .unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.initial_fen, custom_fen);
    }

    #[test]
    fn test_tournament_link() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO tournaments (name) VALUES ('Open')", [])
            .unwrap();
        let tid = conn.last_insert_rowid();
        let id = insert(
            &conn,
            &NewGame {
                tournament_id: Some(tid),
                ..minimal_game()
            },
        )
        .unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.tournament_id, Some(tid));
    }
}
