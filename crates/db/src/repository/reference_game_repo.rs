//! Read repository for the `games` table of the reference games
//! database (PHASE 82) — distinct from [`crate::repository::game_repo`]
//! (application database), which has different columns.
//!
//! Supported filters (decision settled during discussion, PHASE 82, point 7):
//! player (White or Black), Elo range, date/period, opening (ECO).
//! Each filter is optional and combines with the others (logical `AND`).

use std::fmt::Write as _;

use rusqlite::{params_from_iter, Connection, OptionalExtension, Result as SqlResult, ToSql};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Row of the `games` table of the reference database, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceGameRow {
    pub id:          i64,
    pub white:       String,
    pub black:       String,
    pub result:      String,
    pub date:        Option<String>,
    pub event:       Option<String>,
    pub site:        Option<String>,
    pub round:       Option<String>,
    pub eco:         Option<String>,
    pub white_elo:   Option<i64>,
    pub black_elo:   Option<i64>,
    pub white_title: Option<String>,
    pub black_title: Option<String>,
    pub ply_count:   i64,
    pub pgn:         String,
    pub initial_fen: String,
    pub created_at:  String,
}

/// Search criteria for the list of games in the reference database.
///
/// All fields are optional (`None` = no filter on this criterion).
/// `player` is a case-insensitive text search (substring),
/// applied to White OR Black — a player name in an external database can
/// appear in slightly different formats ("Last, First" vs
/// "First Last"), an exact match (like
/// `game_repo::find_by_player` on the application database, deliberately
/// smaller) would be too strict here.
///
/// `min_elo`/`max_elo` filter on **a single given player** (White or
/// Black) whose Elo falls within the range — not "one of the two above the
/// minimum and the other below the maximum" (see [`WHERE_SQL`] for the
/// query detail).
///
/// `eco` filters by prefix (e.g. `"A0"` covers the whole A00-A09 family,
/// `"A00"` retains only this exact code).
#[derive(Debug, Clone, Copy, Default)]
pub struct GameFilter<'a> {
    pub player:    Option<&'a str>,
    pub min_elo:   Option<i64>,
    pub max_elo:   Option<i64>,
    pub date_from: Option<&'a str>,
    pub date_to:   Option<&'a str>,
    pub eco:       Option<&'a str>,
    /// Ergonomics follow-up 10/07/2026 — restricts to games whose identifier
    /// is in this list (the "Opening tree" filter, see
    /// `opening_repo::games_for_path`: list of games that reached a
    /// given position, possibly restricted to the checked following
    /// moves). `None` = no restriction. Combines (logical AND) with
    /// all other criteria, like them — decision settled with
    /// the user: the classic filters (player/Elo/date/ECO)
    /// continue to apply ON TOP OF the subset of games
    /// designated by the opening tree, never one instead of the other.
    pub game_ids: Option<&'a [i64]>,
}

// ---------------------------------------------------------------------------
// `SQL` query
// ---------------------------------------------------------------------------

/// `WHERE` clause shared by [`search`]/[`count_matching`] (copied into the
/// two constants below — there is no `const` concatenation mechanism in
/// stable Rust for `&str`, the slight duplication is accepted).
///
/// Deliberately simple technique: each criterion has the form
/// `(?N IS NULL OR <condition>)`, which keeps the query always valid
/// (no dynamic `SQL` construction) at the cost of less optimal
/// index usage when several filters are combined — acceptable for a
/// first functional version; to revisit if a concrete performance
/// need arises in practice (same principle as the project's existing perf
/// audits, added only when a real problem is observed).
///
/// `?1` = player (substring, White OR Black); `?2`/`?3` = Elo range (see
/// [`GameFilter`], filter on a single given player); `?4`/`?5` = date from/to
/// (lexicographic comparison, consistent with the PGN `YYYY.MM.DD` format);
/// `?6` = ECO prefix.
const WHERE_SQL: &str = "
    (?1 IS NULL OR lower(white) LIKE '%' || lower(?1) || '%'
               OR  lower(black) LIKE '%' || lower(?1) || '%')
    AND (
        (?2 IS NULL AND ?3 IS NULL)
        OR (white_elo IS NOT NULL
            AND white_elo >= IFNULL(?2, -999999) AND white_elo <= IFNULL(?3, 999999))
        OR (black_elo IS NOT NULL
            AND black_elo >= IFNULL(?2, -999999) AND black_elo <= IFNULL(?3, 999999))
    )
    AND (?4 IS NULL OR date >= ?4)
    AND (?5 IS NULL OR date <= ?5)
    AND (?6 IS NULL OR eco LIKE ?6 || '%')
";

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Searches for games matching `filter`, sorted by increasing `id`,
/// with pagination (`limit`/`offset`) — essential on a database that can
/// hold several hundred thousand games (see PHASE 82 discussion
/// on the Gigabase volume).
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn search(
    conn: &Connection,
    filter: &GameFilter<'_>,
    limit: i64,
    offset: i64,
) -> SqlResult<Vec<ReferenceGameRow>> {
    let (where_sql, mut params) = build_where(filter);
    // The `?N` numbers for LIMIT/OFFSET depend on the number of parameters already
    // bound by `build_where` (6 classic filters + possibly as many
    // identifiers as `game_ids` contains) — never fixed at ?7/?8
    // as before the addition of the "Opening tree" filter (ergonomics
    // follow-up 10/07/2026), except precisely when `game_ids` is `None` (6 params,
    // hence ?7/?8: strictly identical behavior to before).
    let limit_ph  = params.len() + 1;
    let offset_ph = params.len() + 2;
    let sql = format!(
        "SELECT id, white, black, result, date, event, site, round, eco,
                white_elo, black_elo, white_title, black_title, ply_count,
                pgn, initial_fen, created_at
         FROM games
         WHERE {where_sql}
         ORDER BY id ASC
         LIMIT ?{limit_ph} OFFSET ?{offset_ph}"
    );
    params.push(Box::new(limit));
    params.push(Box::new(offset));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params), row_to_game)?;
    rows.collect()
}

/// Counts the total number of games matching `filter`, without
/// pagination — used to display "page X / Y" on the GUI side.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn count_matching(conn: &Connection, filter: &GameFilter<'_>) -> SqlResult<i64> {
    let (where_sql, params) = build_where(filter);
    let sql = format!("SELECT COUNT(*) FROM games WHERE {where_sql}");
    conn.query_row(&sql, params_from_iter(params), |row| row.get(0))
}

/// Looks up a game by its `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<ReferenceGameRow>> {
    conn.query_row(
        "SELECT id, white, black, result, date, event, site, round, eco,
                white_elo, black_elo, white_title, black_title, ply_count,
                pgn, initial_fen, created_at
         FROM games WHERE id = ?1",
        [id],
        row_to_game,
    )
    .optional()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Builds, in positional order `?1`..`?6`, the bound parameters for
/// [`WHERE_SQL`] — the 6 classic filters only (`LIMIT`/`OFFSET`
/// and the possible `game_ids` filter are added separately by
/// [`build_where`]/[`search`], which do not have the same number of parameters
/// depending on the call).
fn bind_filter_params(filter: &GameFilter<'_>) -> Vec<Box<dyn ToSql>> {
    vec![
        Box::new(filter.player.map(str::to_string)),
        Box::new(filter.min_elo),
        Box::new(filter.max_elo),
        Box::new(filter.date_from.map(str::to_string)),
        Box::new(filter.date_to.map(str::to_string)),
        Box::new(filter.eco.map(str::to_string)),
    ]
}

/// Builds the full `WHERE` clause (the 6 classic filters from
/// [`WHERE_SQL`], plus a possible restriction by identifiers —
/// [`GameFilter::game_ids`], ergonomics follow-up 10/07/2026: "Opening
/// tree" filter) and the parameters bound in the same order as the `?N` of
/// the produced text.
///
/// When `game_ids` is `None`, the returned text is exactly
/// [`WHERE_SQL`] and the parameters exactly those from [`bind_filter_params`]
/// (6 values, `?1`-`?6`) — behavior strictly identical to that
/// before this filter was added, no regression for callers that don't
/// use it (see the existing tests of [`search`]/[`count_matching`]).
fn build_where(filter: &GameFilter<'_>) -> (String, Vec<Box<dyn ToSql>>) {
    let mut sql = WHERE_SQL.to_string();
    let mut params = bind_filter_params(filter);

    if let Some(ids) = filter.game_ids {
        if ids.is_empty() {
            // Restriction to an empty set: no game can
            // match — always-false condition rather than a
            // syntactically invalid `IN ()`.
            sql.push_str(" AND 0");
        } else {
            let start = params.len() + 1; // 7
            let placeholders: Vec<String> =
                (0..ids.len()).map(|i| format!("?{}", start + i)).collect();
            let _ = write!(sql, " AND id IN ({})", placeholders.join(", "));
            for &id in ids {
                params.push(Box::new(id));
            }
        }
    }

    (sql, params)
}

/// Maps a `SQLite` row to a [`ReferenceGameRow`].
fn row_to_game(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceGameRow> {
    Ok(ReferenceGameRow {
        id:          row.get(0)?,
        white:       row.get(1)?,
        black:       row.get(2)?,
        result:      row.get(3)?,
        date:        row.get(4)?,
        event:       row.get(5)?,
        site:        row.get(6)?,
        round:       row.get(7)?,
        eco:         row.get(8)?,
        white_elo:   row.get(9)?,
        black_elo:   row.get(10)?,
        white_title: row.get(11)?,
        black_title: row.get(12)?,
        ply_count:   row.get(13)?,
        pgn:         row.get(14)?,
        initial_fen: row.get(15)?,
        created_at:  row.get(16)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_import::import_one;
    use crate::reference_schema::open_in_memory;

    /// `elo` groups the White/Black Elo pair into a single parameter so this
    /// test helper stays at 7 arguments (`clippy::too_many_arguments`, max 7).
    fn insert(conn: &Connection, white: &str, black: &str, elo: (Option<i64>, Option<i64>), eco: &str, date: &str, result: &str) {
        let (elo_w, elo_b) = elo;
        let pgn = format!(
            "[Event \"T\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result}\"]\n[Date \"{date}\"]\n[ECO \"{eco}\"]\n{elo_w_tag}{elo_b_tag}\n1. e4 e5 {result}\n",
            elo_w_tag = elo_w.map(|e| format!("[WhiteElo \"{e}\"]\n")).unwrap_or_default(),
            elo_b_tag = elo_b.map(|e| format!("[BlackElo \"{e}\"]\n")).unwrap_or_default(),
        );
        import_one(conn, &pgn).unwrap();
    }

    #[test]
    fn test_search_no_filter_returns_all() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "Alice", "Bob", (Some(2000), Some(1900)), "A00", "2024.01.01", "1-0");
        insert(&conn, "Carol", "Dave", (Some(2500), Some(2400)), "B01", "2024.02.01", "0-1");

        let rows = search(&conn, &GameFilter::default(), 10, 0).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_search_filter_by_player_substring_case_insensitive() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "Carlsen, Magnus", "Nepomniachtchi, Ian", (Some(2839), Some(2792)), "A00", "2024.01.01", "1-0");
        insert(&conn, "Alice", "Bob", (Some(1500), Some(1400)), "A00", "2024.01.01", "1-0");

        let filter = GameFilter { player: Some("carlsen"), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white, "Carlsen, Magnus");
    }

    #[test]
    fn test_search_filter_by_elo_range_matches_single_player_within_range() {
        let conn = open_in_memory().unwrap();
        // A game with one player (2839) above the range, the other (2792) inside it.
        insert(&conn, "Carlsen, Magnus", "Nepomniachtchi, Ian", (Some(2839), Some(2792)), "A00", "2024.01.01", "1-0");
        // A game where neither player falls within the requested range.
        insert(&conn, "Alice", "Bob", (Some(1500), Some(1400)), "A00", "2024.01.01", "1-0");

        let filter = GameFilter { min_elo: Some(2700), max_elo: Some(2800), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].black_elo, Some(2792));
    }

    #[test]
    fn test_search_filter_by_date_range() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2020.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "A00", "2026.06.01", "1-0");

        let filter = GameFilter { date_from: Some("2025.01.01"), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white, "C");
    }

    #[test]
    fn test_search_filter_by_eco_prefix() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "A45", "2024.01.01", "1-0");
        insert(&conn, "E", "F", (None, None), "B01", "2024.01.01", "1-0");

        let filter = GameFilter { eco: Some("A"), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_search_pagination_limit_and_offset() {
        let conn = open_in_memory().unwrap();
        for i in 0..5 {
            insert(&conn, &format!("W{i}"), "B", (None, None), "A00", "2024.01.01", "1-0");
        }
        let page1 = search(&conn, &GameFilter::default(), 2, 0).unwrap();
        let page2 = search(&conn, &GameFilter::default(), 2, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id);
    }

    #[test]
    fn test_count_matching_respects_same_filter_as_search() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "B01", "2024.01.01", "1-0");

        let filter = GameFilter { eco: Some("A"), ..Default::default() };
        let count = count_matching(&conn, &filter).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_find_by_id_found_and_not_found() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        let row = find_by_id(&conn, 1).unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap().white, "A");

        assert!(find_by_id(&conn, 9999).unwrap().is_none());
    }

    // -- Ergonomics follow-up 10/07/2026: "Opening tree" filter (game_ids) --

    #[test]
    fn test_search_filter_by_game_ids_restricts_to_listed_ids() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "E", "F", (None, None), "A00", "2024.01.01", "1-0");

        let ids = [1i64, 3i64];
        let filter = GameFilter { game_ids: Some(&ids), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].white, "A");
        assert_eq!(rows[1].white, "E");
    }

    #[test]
    fn test_search_filter_by_game_ids_combines_with_other_filters() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "B01", "2024.01.01", "1-0");

        // id 1 is part of the selection but its ECO does not match
        // the classic filter applied on top: logical AND, no game
        // should match.
        let ids = [1i64];
        let filter = GameFilter { game_ids: Some(&ids), eco: Some("B"), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_search_filter_by_empty_game_ids_returns_nothing() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");

        let ids: [i64; 0] = [];
        let filter = GameFilter { game_ids: Some(&ids), ..Default::default() };
        let rows = search(&conn, &filter, 10, 0).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_count_matching_respects_game_ids_filter() {
        let conn = open_in_memory().unwrap();
        insert(&conn, "A", "B", (None, None), "A00", "2024.01.01", "1-0");
        insert(&conn, "C", "D", (None, None), "A00", "2024.01.01", "1-0");

        let ids = [2i64];
        let filter = GameFilter { game_ids: Some(&ids), ..Default::default() };
        let count = count_matching(&conn, &filter).unwrap();
        assert_eq!(count, 1);
    }
}
