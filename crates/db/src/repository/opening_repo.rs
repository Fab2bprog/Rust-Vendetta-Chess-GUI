//! Opening tree queries (PHASE 82): next moves and
//! statistics from a given position, aggregated on the fly over
//! `game_positions` (see [`crate::reference_schema`] for the reasoning
//! behind not using a pre-aggregated table).
//!
//! The minimum Elo threshold is a **query parameter**, not data
//! fixed at import time: this is the condition raised during discussion (PHASE 82,
//! point 5) so the user can adjust it without ever having to
//! reimport the database.

use std::fmt::Write as _;

use rusqlite::{Connection, Result as SqlResult};

use crate::reference_schema::hash_to_sql;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Aggregated statistics for a move played from a given position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningMoveStats {
    /// Move played, in UCI format (e.g. `"e2e4"`).
    pub uci_move:   String,
    /// Number of games in the database that played this move from this position
    /// (after applying the Elo threshold, see [`next_moves`]).
    pub games:      i64,
    /// Number of White wins among these games.
    pub white_wins: i64,
    /// Number of draws among these games.
    pub draws:      i64,
    /// Number of Black wins among these games.
    pub black_wins: i64,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Returns, for the Polyglot hash position `position_hash`, the list of
/// moves played in the database (with their statistics), sorted by
/// decreasing number of games (most popular move first).
///
/// `min_elo` filters the games counted: `None` = all games
/// count (no filter); `Some(threshold)` excludes games where **both**
/// players have a known Elo strictly below the threshold.
/// **Decision settled during discussion (PHASE 82)**: a game where neither
/// player has an Elo on record is **always counted**, regardless of
/// `min_elo` — the threshold only excludes games where the level is *known*
/// to be insufficient, never those where it is unknown.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn next_moves(
    conn: &Connection,
    position_hash: u64,
    min_elo: Option<i64>,
) -> SqlResult<Vec<OpeningMoveStats>> {
    let hash = hash_to_sql(position_hash);

    let mut stmt = conn.prepare(
        "SELECT gp.uci_move,
                COUNT(*) AS games,
                SUM(CASE WHEN g.result = '1-0'     THEN 1 ELSE 0 END) AS white_wins,
                SUM(CASE WHEN g.result = '1/2-1/2' THEN 1 ELSE 0 END) AS draws,
                SUM(CASE WHEN g.result = '0-1'     THEN 1 ELSE 0 END) AS black_wins
         FROM game_positions gp
         JOIN games g ON g.id = gp.game_id
         WHERE gp.position_hash = ?1
           AND (
                ?2 IS NULL
                OR (g.white_elo IS NULL AND g.black_elo IS NULL)
                OR g.white_elo >= ?2
                OR g.black_elo >= ?2
           )
         GROUP BY gp.uci_move
         ORDER BY games DESC, gp.uci_move ASC",
    )?;

    let rows = stmt.query_map(rusqlite::params![hash, min_elo], |row| {
        Ok(OpeningMoveStats {
            uci_move:   row.get(0)?,
            games:      row.get(1)?,
            white_wins: row.get(2)?,
            draws:      row.get(3)?,
            black_wins: row.get(4)?,
        })
    })?;

    rows.collect()
}

/// Returns the identifiers (`games.id`) of the games in the database that
/// reached the Polyglot hash position `position_hash` (ergonomics follow-up
/// 10/07/2026: "List games" button of the opening tree — a
/// missing bridge between the aggregated statistics from [`next_moves`] and the
/// individual games that make them up, reported by the user).
///
/// `allowed_moves` restricts to games that subsequently played one of the
/// listed UCI moves (checkboxes in the candidate moves table):
/// `None` = no restriction, all games that reached this
/// position count (default behavior when no checkbox is
/// checked — the same set as the total already displayed by [`next_moves`]).
/// An empty slice is not expected to be passed by the caller (`None` is the
/// canonical way to express "no restriction"); still handled
/// as "no game can match" rather than producing an invalid
/// `IN ()`.
///
/// `min_elo`: same semantics as [`next_moves`] (a game with no Elo
/// on record at all always counts).
///
/// Sorted by increasing `id`, with no pagination: this list is only used to
/// feed `reference_game_repo::GameFilter::game_ids`, which in turn paginates
/// on the SQL side — no limit here so as not to silently truncate
/// the set before that final filtering.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn games_for_path(
    conn: &Connection,
    position_hash: u64,
    allowed_moves: Option<&[String]>,
    min_elo: Option<i64>,
) -> SqlResult<Vec<i64>> {
    let hash = hash_to_sql(position_hash);

    let mut sql = String::from(
        "SELECT DISTINCT gp.game_id
         FROM game_positions gp
         JOIN games g ON g.id = gp.game_id
         WHERE gp.position_hash = ?1
           AND (
                ?2 IS NULL
                OR (g.white_elo IS NULL AND g.black_elo IS NULL)
                OR g.white_elo >= ?2
                OR g.black_elo >= ?2
           )",
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(hash), Box::new(min_elo)];

    if let Some(moves) = allowed_moves {
        if moves.is_empty() {
            // Case normally never reached by the caller (see doc) —
            // safe fallback: condition always false rather than a
            // syntactically invalid `IN ()`.
            sql.push_str(" AND 0");
        } else {
            let start = params.len() + 1; // 3
            let placeholders: Vec<String> =
                (0..moves.len()).map(|i| format!("?{}", start + i)).collect();
            let _ = write!(sql, " AND gp.uci_move IN ({})", placeholders.join(", "));
            for m in moves {
                params.push(Box::new(m.clone()));
            }
        }
    }

    sql.push_str(" ORDER BY gp.game_id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| row.get(0))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_import::import_one;
    use crate::reference_schema::open_in_memory;
    use core::polyglot::polyglot_hash;
    use core::types::Position;

    fn insert_game(conn: &Connection, moves: &str, result: &str, white_elo: Option<i64>, black_elo: Option<i64>) {
        let white_elo_tag = white_elo.map(|e| format!("[WhiteElo \"{e}\"]\n")).unwrap_or_default();
        let black_elo_tag = black_elo.map(|e| format!("[BlackElo \"{e}\"]\n")).unwrap_or_default();
        let pgn = format!(
            "[Event \"T\"]\n[White \"A\"]\n[Black \"B\"]\n[Result \"{result}\"]\n{white_elo_tag}{black_elo_tag}\n{moves} {result}\n"
        );
        import_one(conn, &pgn).unwrap();
    }

    fn starting_hash() -> u64 {
        polyglot_hash(&Position::starting())
    }

    #[test]
    fn test_next_moves_aggregates_across_games_no_filter() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);
        insert_game(&conn, "1. e4 e5", "0-1", None, None);
        insert_game(&conn, "1. d4 d5", "1/2-1/2", None, None);

        let moves = next_moves(&conn, starting_hash(), None).unwrap();
        assert_eq!(moves.len(), 2);

        let e4 = moves.iter().find(|m| m.uci_move == "e2e4").unwrap();
        assert_eq!(e4.games, 2);
        assert_eq!(e4.white_wins, 1);
        assert_eq!(e4.black_wins, 1);

        let d4 = moves.iter().find(|m| m.uci_move == "d2d4").unwrap();
        assert_eq!(d4.games, 1);
        assert_eq!(d4.draws, 1);
    }

    #[test]
    fn test_next_moves_ordered_by_popularity_descending() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);
        insert_game(&conn, "1. e4 e5", "1-0", None, None);
        insert_game(&conn, "1. d4 d5", "1-0", None, None);

        let moves = next_moves(&conn, starting_hash(), None).unwrap();
        assert_eq!(moves[0].uci_move, "e2e4");
        assert_eq!(moves[0].games, 2);
    }

    #[test]
    fn test_next_moves_min_elo_excludes_known_low_elo_games() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", Some(1200), Some(1100)); // below the threshold
        insert_game(&conn, "1. e4 e5", "0-1", Some(2600), Some(2500)); // above the threshold

        let moves = next_moves(&conn, starting_hash(), Some(2000)).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].games, 1);
        assert_eq!(moves[0].black_wins, 1);
    }

    #[test]
    fn test_next_moves_min_elo_still_counts_games_without_any_elo() {
        // Decision settled PHASE 82: a game with no Elo on record at all
        // always counts, even with a high minimum threshold.
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);

        let moves = next_moves(&conn, starting_hash(), Some(2800)).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].games, 1);
    }

    #[test]
    fn test_next_moves_min_elo_counts_game_when_only_one_side_rated_above() {
        let conn = open_in_memory().unwrap();
        // Only one player has a known and sufficient Elo, the other unrated.
        insert_game(&conn, "1. e4 e5", "1-0", Some(2700), None);

        let moves = next_moves(&conn, starting_hash(), Some(2000)).unwrap();
        assert_eq!(moves.len(), 1);
    }

    #[test]
    fn test_next_moves_empty_for_unknown_position() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);

        // Arbitrary hash matching no indexed position.
        let moves = next_moves(&conn, 0xdead_beef_dead_beef, None).unwrap();
        assert!(moves.is_empty());
    }

    // ── games_for_path (ergonomics follow-up 10/07/2026) ────────────────────

    #[test]
    fn test_games_for_path_no_restriction_returns_all_games_at_position() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);
        insert_game(&conn, "1. e4 e5", "0-1", None, None);
        insert_game(&conn, "1. d4 d5", "1/2-1/2", None, None);

        let ids = games_for_path(&conn, starting_hash(), None, None).unwrap();
        assert_eq!(ids.len(), 3, "les 3 parties ont toutes joué un premier coup depuis le départ");
    }

    #[test]
    fn test_games_for_path_restricts_to_allowed_moves() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);
        insert_game(&conn, "1. e4 e5", "0-1", None, None);
        insert_game(&conn, "1. d4 d5", "1/2-1/2", None, None);

        let allowed = vec!["e2e4".to_string()];
        let ids = games_for_path(&conn, starting_hash(), Some(&allowed), None).unwrap();
        assert_eq!(ids.len(), 2, "seules les 2 parties ayant joué 1.e4 doivent être retenues");
    }

    #[test]
    fn test_games_for_path_empty_allowed_moves_returns_nothing() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);

        let allowed: Vec<String> = vec![];
        let ids = games_for_path(&conn, starting_hash(), Some(&allowed), None).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_games_for_path_respects_min_elo_like_next_moves() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", Some(1200), Some(1100)); // below
        insert_game(&conn, "1. e4 e5", "0-1", Some(2600), Some(2500)); // above

        let ids = games_for_path(&conn, starting_hash(), None, Some(2000)).unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_games_for_path_empty_for_unknown_position() {
        let conn = open_in_memory().unwrap();
        insert_game(&conn, "1. e4 e5", "1-0", None, None);

        let ids = games_for_path(&conn, 0xdead_beef_dead_beef, None, None).unwrap();
        assert!(ids.is_empty());
    }
}
