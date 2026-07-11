//! Repository for the `tournaments` table and tournament games.
//!
//! All functions receive a `&Connection`; transaction management
//! is left to the caller.
//!
//! ## Overview
//!
//! - [`create_tournament`]  : creates a tournament, returns its `id`.
//! - [`save_game_result`]   : inserts a game linked to a tournament.
//! - [`get_standings`]      : W/D/L standings aggregated from `games`.
//! - [`find_by_id`]         : looks up a tournament by its `id`.
//! - [`find_all`]           : lists all tournaments.
//! - [`game_count`]         : number of games played in a tournament.

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Row of the `tournaments` table.
#[derive(Debug, Clone, PartialEq)]
pub struct TournamentRow {
    pub id:         i64,
    pub name:       String,
    pub site:       Option<String>,
    pub date:       Option<String>,
    /// "roundrobin" or "gauntlet".
    pub kind:       String,
    pub created_at: String,
}

/// Standings row for an engine in a tournament.
///
/// Computed via a SQL aggregate on the `games` table.
#[derive(Debug, Clone, PartialEq)]
pub struct StandingRow {
    /// Engine name (`white` or `black` column in `games`).
    pub engine_name: String,
    /// FIDE points: win = 1.0, draw = 0.5, loss = 0.0.
    pub points:      f64,
    pub wins:        i64,
    pub draws:       i64,
    pub losses:      i64,
    /// Total number of games played.
    pub games:       i64,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Inserts a new tournament and returns its auto-incremented `id`.
///
/// `kind` must be `"roundrobin"` or `"gauntlet"`.
///
/// # Errors
///
/// Returns a `SQLite` error if the database is locked.
pub fn create_tournament(conn: &Connection, name: &str, kind: &str) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO tournaments (name, kind) VALUES (?1, ?2)",
        params![name, kind],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Inserts a tournament game into the `games` table.
///
/// `result` must be `"1-0"`, `"0-1"`, or `"1/2-1/2"`.
/// `round` is the round number (1-based), stored as text.
///
/// Returns the `id` of the inserted game.
///
/// # Errors
///
/// Returns a `SQLite` error if `tournament_id` does not exist (foreign key).
///
/// Clippy (04/07/2026): `#[allow(too_many_arguments)]` — 8 simple
/// parameters, already named/aligned, each corresponding to a distinct
/// column of the `games` table; grouping them into a struct would add a layer
/// of indirection for this single caller, with no real robustness gain.
#[allow(clippy::too_many_arguments)]
pub fn save_game_result(
    conn:          &Connection,
    tournament_id: i64,
    white:         &str,
    black:         &str,
    result:        &str,
    pgn:           &str,
    round:         u32,
    move_count:    i64,
) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO games
             (tournament_id, white, black, result, pgn, round, move_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            tournament_id,
            white,
            black,
            result,
            pgn,
            round.to_string(),
            move_count,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Returns the standings for all games of tournament `tournament_id`,
/// sorted by decreasing points then decreasing wins.
///
/// Uses a SQL aggregate on the `games` table — no dependency on
/// the `tournament` crate.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn get_standings(conn: &Connection, tournament_id: i64) -> SqlResult<Vec<StandingRow>> {
    let sql = "
        SELECT
            engine_name,
            CAST(SUM(points) AS REAL) AS total_points,
            SUM(wins)   AS wins,
            SUM(draws)  AS draws,
            SUM(losses) AS losses,
            COUNT(*)    AS games
        FROM (
            -- White view
            SELECT
                white AS engine_name,
                CASE result
                    WHEN '1-0'     THEN 1.0
                    WHEN '1/2-1/2' THEN 0.5
                    ELSE                 0.0
                END AS points,
                CASE result WHEN '1-0'     THEN 1 ELSE 0 END AS wins,
                CASE result WHEN '1/2-1/2' THEN 1 ELSE 0 END AS draws,
                CASE result WHEN '0-1'     THEN 1 ELSE 0 END AS losses
            FROM games WHERE tournament_id = ?1

            UNION ALL

            -- Black view
            SELECT
                black AS engine_name,
                CASE result
                    WHEN '0-1'     THEN 1.0
                    WHEN '1/2-1/2' THEN 0.5
                    ELSE                 0.0
                END AS points,
                CASE result WHEN '0-1'     THEN 1 ELSE 0 END AS wins,
                CASE result WHEN '1/2-1/2' THEN 1 ELSE 0 END AS draws,
                CASE result WHEN '1-0'     THEN 1 ELSE 0 END AS losses
            FROM games WHERE tournament_id = ?1
        )
        GROUP BY engine_name
        ORDER BY total_points DESC, wins DESC
    ";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([tournament_id], |row| {
        Ok(StandingRow {
            engine_name: row.get(0)?,
            points:      row.get(1)?,
            wins:        row.get(2)?,
            draws:       row.get(3)?,
            losses:      row.get(4)?,
            games:       row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Looks up a tournament by its `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<TournamentRow>> {
    conn.query_row(
        "SELECT id, name, site, date, kind, created_at FROM tournaments WHERE id = ?1",
        [id],
        row_to_tournament,
    )
    .optional()
}

/// Returns all tournaments, sorted by decreasing `id` (most recent first).
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_all(conn: &Connection) -> SqlResult<Vec<TournamentRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, site, date, kind, created_at FROM tournaments ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], row_to_tournament)?;
    rows.collect()
}

/// Number of games recorded for a given tournament.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn game_count(conn: &Connection, tournament_id: i64) -> SqlResult<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM games WHERE tournament_id = ?1",
        [tournament_id],
        |row| row.get(0),
    )
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn row_to_tournament(row: &rusqlite::Row<'_>) -> rusqlite::Result<TournamentRow> {
    Ok(TournamentRow {
        id:         row.get(0)?,
        name:       row.get(1)?,
        site:       row.get(2)?,
        date:       row.get(3)?,
        kind:       row.get(4)?,
        created_at: row.get(5)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
// Clippy (04/07/2026): `#[allow(float_cmp)]` — the `.points` compared here are
// exact sums of 0.0/0.5/1.0/2.0 (deterministic tournament scores, not
// a rounded floating-point computation), as in `tournament::lib`, already audited.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::schema::open_in_memory;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Creates a Round Robin tournament and returns its id.
    fn setup_tournament(conn: &Connection) -> i64 {
        create_tournament(conn, "Test Open", "roundrobin").unwrap()
    }

    /// Inserts a tournament game with minimal values.
    fn add_game(conn: &Connection, tid: i64, white: &str, black: &str, result: &str) -> i64 {
        save_game_result(conn, tid, white, black, result, "", 1, 20).unwrap()
    }

    // ── create_tournament ─────────────────────────────────────────────────────

    #[test]
    fn test_create_tournament_returns_id() {
        let conn = open_in_memory().unwrap();
        let id = create_tournament(&conn, "Open A", "roundrobin").unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_create_tournament_increments_id() {
        let conn = open_in_memory().unwrap();
        let id1 = create_tournament(&conn, "T1", "roundrobin").unwrap();
        let id2 = create_tournament(&conn, "T2", "gauntlet").unwrap();
        assert!(id2 > id1);
    }

    // ── find_by_id ────────────────────────────────────────────────────────────

    #[test]
    fn test_find_by_id_found() {
        let conn = open_in_memory().unwrap();
        let id = create_tournament(&conn, "My Cup", "gauntlet").unwrap();
        let row = find_by_id(&conn, id).unwrap().unwrap();
        assert_eq!(row.id,   id);
        assert_eq!(row.name, "My Cup");
        assert_eq!(row.kind, "gauntlet");
    }

    #[test]
    fn test_find_by_id_not_found() {
        let conn = open_in_memory().unwrap();
        assert!(find_by_id(&conn, 9999).unwrap().is_none());
    }

    // ── find_all ──────────────────────────────────────────────────────────────

    #[test]
    fn test_find_all_empty() {
        let conn = open_in_memory().unwrap();
        assert!(find_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn test_find_all_returns_most_recent_first() {
        let conn = open_in_memory().unwrap();
        create_tournament(&conn, "T1", "roundrobin").unwrap();
        create_tournament(&conn, "T2", "roundrobin").unwrap();
        let all = find_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "T2"); // decreasing id
        assert_eq!(all[1].name, "T1");
    }

    // ── save_game_result ─────────────────────────────────────────────────────

    #[test]
    fn test_save_game_result_returns_id() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        let gid = save_game_result(&conn, tid, "E0", "E1", "1-0", "", 1, 30).unwrap();
        assert!(gid > 0);
    }

    #[test]
    fn test_save_game_invalid_tournament_fails() {
        let conn = open_in_memory().unwrap();
        // open_in_memory does not enable FKs — enable them explicitly
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let result = save_game_result(&conn, 9999, "E0", "E1", "1-0", "", 1, 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_save_game_stores_round() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        save_game_result(&conn, tid, "E0", "E1", "1-0", "", 3, 25).unwrap();
        // Verify by reading directly from games
        let round: String = conn.query_row(
            "SELECT round FROM games WHERE tournament_id = ?1",
            [tid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(round, "3");
    }

    // ── game_count ────────────────────────────────────────────────────────────

    #[test]
    fn test_game_count_zero() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        assert_eq!(game_count(&conn, tid).unwrap(), 0);
    }

    #[test]
    fn test_game_count_increments() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "1-0");
        add_game(&conn, tid, "E1", "E0", "0-1");
        assert_eq!(game_count(&conn, tid).unwrap(), 2);
    }

    #[test]
    fn test_game_count_isolated_per_tournament() {
        let conn = open_in_memory().unwrap();
        let tid1 = setup_tournament(&conn);
        let tid2 = create_tournament(&conn, "T2", "gauntlet").unwrap();
        add_game(&conn, tid1, "E0", "E1", "1-0");
        add_game(&conn, tid1, "E0", "E1", "1-0");
        add_game(&conn, tid2, "E0", "E2", "1/2-1/2");
        assert_eq!(game_count(&conn, tid1).unwrap(), 2);
        assert_eq!(game_count(&conn, tid2).unwrap(), 1);
    }

    // ── get_standings ─────────────────────────────────────────────────────────

    #[test]
    fn test_standings_empty_tournament() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        let s = get_standings(&conn, tid).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn test_standings_white_win() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "1-0");
        let s = get_standings(&conn, tid).unwrap();
        assert_eq!(s.len(), 2);
        // E0 wins → 1.0 pt, E1 loses → 0.0 pt
        let e0 = s.iter().find(|r| r.engine_name == "E0").unwrap();
        let e1 = s.iter().find(|r| r.engine_name == "E1").unwrap();
        assert_eq!(e0.points, 1.0);
        assert_eq!(e0.wins,   1);
        assert_eq!(e0.draws,  0);
        assert_eq!(e0.losses, 0);
        assert_eq!(e1.points, 0.0);
        assert_eq!(e1.wins,   0);
        assert_eq!(e1.losses, 1);
    }

    #[test]
    fn test_standings_black_win() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "0-1");
        let s = get_standings(&conn, tid).unwrap();
        let e0 = s.iter().find(|r| r.engine_name == "E0").unwrap();
        let e1 = s.iter().find(|r| r.engine_name == "E1").unwrap();
        assert_eq!(e0.losses, 1);
        assert_eq!(e0.points, 0.0);
        assert_eq!(e1.wins,   1);
        assert_eq!(e1.points, 1.0);
    }

    #[test]
    fn test_standings_draw() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "1/2-1/2");
        let s = get_standings(&conn, tid).unwrap();
        for row in &s {
            assert_eq!(row.points, 0.5);
            assert_eq!(row.draws,  1);
            assert_eq!(row.wins,   0);
            assert_eq!(row.losses, 0);
        }
    }

    #[test]
    fn test_standings_sorted_by_points_desc() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        // E0 beats E1 and E2; E1 beats E2
        add_game(&conn, tid, "E0", "E1", "1-0");
        add_game(&conn, tid, "E0", "E2", "1-0");
        add_game(&conn, tid, "E1", "E2", "1-0");
        let s = get_standings(&conn, tid).unwrap();
        assert_eq!(s[0].engine_name, "E0");
        assert_eq!(s[0].points,      2.0);
        assert_eq!(s[1].engine_name, "E1");
        assert_eq!(s[1].points,      1.0);
        assert_eq!(s[2].engine_name, "E2");
        assert_eq!(s[2].points,      0.0);
    }

    #[test]
    fn test_standings_tiebreak_by_wins() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        // E0 wins one, loses one → 1 pt, 1W
        add_game(&conn, tid, "E0", "E1", "1-0");
        add_game(&conn, tid, "E2", "E0", "1-0");
        // E1 draws twice → 1 pt, 0W
        add_game(&conn, tid, "E1", "E2", "1/2-1/2");
        add_game(&conn, tid, "E2", "E1", "1/2-1/2");
        let s = get_standings(&conn, tid).unwrap();
        // E0 and E1 have 1 pt but E0 has 1W > 0W
        let e0 = s.iter().position(|r| r.engine_name == "E0").unwrap();
        let e1 = s.iter().position(|r| r.engine_name == "E1").unwrap();
        assert!(e0 < e1);
    }

    #[test]
    fn test_standings_games_count() {
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "1-0");
        add_game(&conn, tid, "E1", "E0", "0-1");
        let s = get_standings(&conn, tid).unwrap();
        for row in &s {
            assert_eq!(row.games, 2);
        }
    }

    #[test]
    fn test_standings_isolated_from_other_tournament() {
        let conn = open_in_memory().unwrap();
        let tid1 = setup_tournament(&conn);
        let tid2 = create_tournament(&conn, "T2", "gauntlet").unwrap();
        add_game(&conn, tid1, "E0", "E1", "1-0");
        add_game(&conn, tid2, "E2", "E3", "0-1"); // other tournament
        let s = get_standings(&conn, tid1).unwrap();
        // tid1 must only see E0 and E1
        assert_eq!(s.len(), 2);
        assert!(s.iter().all(|r| r.engine_name == "E0" || r.engine_name == "E1"));
    }

    #[test]
    fn test_standings_three_engine_rr_all_wins_white() {
        // RR 3 engines, 1 game per pair, white always wins
        // E0 vs E1 (1-0), E0 vs E2 (1-0), E1 vs E2 (1-0)
        let conn = open_in_memory().unwrap();
        let tid = setup_tournament(&conn);
        add_game(&conn, tid, "E0", "E1", "1-0");
        add_game(&conn, tid, "E0", "E2", "1-0");
        add_game(&conn, tid, "E1", "E2", "1-0");
        let s = get_standings(&conn, tid).unwrap();
        let e0 = s.iter().find(|r| r.engine_name == "E0").unwrap();
        let e1 = s.iter().find(|r| r.engine_name == "E1").unwrap();
        let e2 = s.iter().find(|r| r.engine_name == "E2").unwrap();
        assert_eq!(e0.points, 2.0);
        assert_eq!(e1.points, 1.0);
        assert_eq!(e2.points, 0.0);
        assert_eq!(e0.wins,   2);
        assert_eq!(e1.wins,   1);
        assert_eq!(e2.wins,   0);
    }
}
