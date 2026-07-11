//! PGN import to the reference games database (PHASE 82).
//!
//! Deliberate differences from [`crate::import_export`] (PGN import
//! to the application database):
//!
//! - **Enriched metadata**: in addition to White/Black/Result/Date/Event/
//!   Site/Round, this module extracts `ECO`, `WhiteElo`, `BlackElo`,
//!   `WhiteTitle`, `BlackTitle` — present in external PGN databases like
//!   Lumbra's Gigabase, needed for the game list filters
//!   and for the adjustable Elo threshold of the opening tree (see
//!   [`crate::reference_schema`]).
//! - **PGN kept as-is, never regenerated**: unlike
//!   `import_export::import_pgn_to_db` (which regenerates a canonical PGN from
//!   [`core::pgn::PgnTags`], which only carries 7 fields), this module stores
//!   the PGN text **exactly as provided as input** for each
//!   game. An external database like Lumbra contains analysis
//!   comments already embedded in the text (e.g. `{ Inaccuracy. Bb4 was best. }`,
//!   observed in the file provided by the user): regenerating them via
//!   `PgnTags` would purely and simply lose them, whereas they are
//!   directly reusable (at zero cost) by the exploration screen.
//! - **Opening tree indexing**: each imported game is
//!   also decomposed into `game_positions` rows (up to
//!   [`crate::reference_schema::OPENING_TREE_MAX_PLIES`] half-moves), via the
//!   Polyglot hash of each position traversed.
//! - **Progress callback**: modeled on
//!   [`crate::repository::puzzle_repo::import_csv_with_progress`] (PHASE 14,
//!   bugfix 03/07/2026) — a large PGN file (the example provided by
//!   the user is 312 MB, ~142,683 games) can take long enough
//!   to look like a crash without visual feedback.

use std::path::Path;

use rusqlite::{Connection, Result as SqlResult, ToSql};

use core::pgn::{self as core_pgn, PgnError};
use core::polyglot::polyglot_hash;
#[cfg(test)]
use core::types::Position;

use crate::import_export::{extract_tag, split_pgn_games};
use crate::reference_schema::{hash_to_sql, OPENING_TREE_MAX_PLIES};

// ---------------------------------------------------------------------------
// Progress notification frequency
// ---------------------------------------------------------------------------

/// Frequency (in number of games processed) at which
/// [`import_pgn_file_with_progress`] notifies its progress callback.
///
/// Bugfix 09/07/2026 (user feedback: "the status blinks but no
/// record scrolling, unlike puzzles"): the initial value (500) did not
/// trigger any visible update for a test import of a few hundred games —
/// the callback only ran once, right at the end, leaving the static
/// "Import in progress…" text displayed (with just the opacity pulse) for the
/// whole duration. Value lowered to 25 for genuinely continuous visual
/// feedback, including on small test files.
///
/// Raised back to 100 (perf bugfix 09/07/2026, following the fast import path
/// via [`core::pgn::import_pgn_trusted`]): each game is now
/// noticeably cheaper to process, a step of 25 would scroll the counter
/// too fast to be readable on a large file — 100 remains largely
/// frequent enough for continuous visual feedback, including on small
/// test files.
/// `pub(crate)` (rather than private) since 12/07/2026: reused as-is
/// by `scid_import::import_si4_file_with_progress` so that the
/// two imports (PGN and SCID) share the same refresh step —
/// a single source of truth rather than a duplicated constant liable
/// to diverge.
pub(crate) const PROGRESS_EVERY: usize = 100;

/// Frequency (in number of games processed) at which
/// [`import_pgn_file_with_progress`] commits (`COMMIT`) its current batch of
/// insertions and opens a new one, rather than grouping everything into a single
/// transaction covering the whole file (bugfix 09/07/2026, see the
/// function's documentation for the full reasoning).
///
/// `pub(crate)` since 12/07/2026 (V2 Phase A1, task #18): reused
/// as-is by `scid_import::import_si4_file_with_progress`, for the
/// same reason as [`PROGRESS_EVERY`] above — a single source of truth.
pub(crate) const COMMIT_EVERY: usize = 1000;

// ---------------------------------------------------------------------------
// Error and summary types
// ---------------------------------------------------------------------------

/// Error while importing a game into the reference database.
#[derive(Debug)]
pub enum ReferenceImportError {
    /// The PGN is invalid or illegal.
    Pgn(PgnError),
    /// `SQLite` error.
    Sql(rusqlite::Error),
    /// File read error.
    Io(std::io::Error),
}

impl std::fmt::Display for ReferenceImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pgn(e) => write!(f, "PGN invalide : {e}"),
            Self::Sql(e) => write!(f, "Erreur base de données : {e}"),
            Self::Io(e)  => write!(f, "Erreur lecture fichier : {e}"),
        }
    }
}

impl From<rusqlite::Error> for ReferenceImportError {
    fn from(e: rusqlite::Error) -> Self { Self::Sql(e) }
}

impl From<std::io::Error> for ReferenceImportError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

/// Summary of a complete import of a PGN file into the reference database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportSummary {
    /// Number of games imported successfully.
    pub imported: usize,
    /// Number of games skipped (invalid/illegal PGN).
    pub skipped:  usize,
}

// ---------------------------------------------------------------------------
// Import of a single game
// ---------------------------------------------------------------------------

/// Parses a PGN, validates the moves, inserts the game (enriched metadata +
/// original PGN text kept as-is), and indexes its positions in
/// `game_positions`, up to `OPENING_TREE_MAX_PLIES` half-moves.
///
/// Returns the auto-incremented `id` of the row inserted into `games`.
///
/// # Performance (bugfix 09/07/2026)
///
/// Uses [`core_pgn::import_pgn_trusted`] rather than [`core_pgn::import_pgn`]:
/// this database is fed by already-published external PGN files
/// (actually played games, e.g. Lumbra's Gigabase), not entered move by
/// move by the user — revalidating each move with the same rigor as an
/// interactive game (full list of legal moves regenerated several
/// times per move, only to then compare SAN text) took up
/// most of the import time for zero practical benefit. See the
/// documentation of `import_pgn_trusted` in `core::pgn` for detail.
///
/// # Errors
///
/// - [`ReferenceImportError::Pgn`] if the PGN is malformed or contains an illegal move.
/// - [`ReferenceImportError::Sql`] if the insertion fails.
pub fn import_one(conn: &Connection, pgn: &str) -> Result<i64, ReferenceImportError> {
    let replay = core_pgn::import_pgn_trusted(pgn, OPENING_TREE_MAX_PLIES)
        .map_err(ReferenceImportError::Pgn)?;

    let white       = extract_tag(pgn, "White").unwrap_or_else(|| "?".to_string());
    let black       = extract_tag(pgn, "Black").unwrap_or_else(|| "?".to_string());
    let result      = extract_tag(pgn, "Result").unwrap_or_else(|| "*".to_string());
    let date        = extract_tag(pgn, "Date");
    let event       = extract_tag(pgn, "Event");
    let site        = extract_tag(pgn, "Site");
    let round       = extract_tag(pgn, "Round");
    let eco         = extract_tag(pgn, "ECO");
    let white_elo   = extract_tag(pgn, "WhiteElo").and_then(|s| s.trim().parse::<i64>().ok());
    let black_elo   = extract_tag(pgn, "BlackElo").and_then(|s| s.trim().parse::<i64>().ok());
    let white_title = extract_tag(pgn, "WhiteTitle");
    let black_title = extract_tag(pgn, "BlackTitle");

    // `ply_count`/`initial_fen` come directly from `import_pgn_trusted`
    // (SAN token counting + standard starting position, no superfluous
    // check computation — bugfix 09/07/2026, see discussion: these two
    // fields never needed full move validation).
    let ply_count   = i64::try_from(replay.ply_count).unwrap_or(i64::MAX);
    let initial_fen = replay.initial_fen.clone();

    conn.execute(
        "INSERT INTO games
             (white, black, result, date, event, site, round, eco,
              white_elo, black_elo, white_title, black_title,
              ply_count, pgn, initial_fen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            white, black, result, date, event, site, round, eco,
            white_elo, black_elo, white_title, black_title,
            ply_count, pgn, initial_fen,
        ],
    )?;
    let game_id = conn.last_insert_rowid();

    index_opening_positions(conn, game_id, &replay)?;

    Ok(game_id)
}

/// Indexes the positions of an imported game in `game_positions` — one
/// row per half-move already resolved by [`core_pgn::import_pgn_trusted`]
/// (bounded to `OPENING_TREE_MAX_PLIES`, see the documentation of
/// [`crate::reference_schema`] for the reasoning behind this limit).
///
/// # Performance (bugfix 09/07/2026)
///
/// A single multi-row `INSERT` statement rather than one execution per
/// half-move (up to 30 round trips per game previously) — the values
/// are accumulated in memory (`Vec`) then written in a single block. Uses
/// `trusted_ply.position_before` directly (already a `Position` in
/// memory, produced by replaying the game) rather than reparsing a text
/// FEN as the old implementation did — avoids
/// an entirely unnecessary Position → FEN → Position round trip.
fn index_opening_positions(
    conn: &Connection,
    game_id: i64,
    replay: &core_pgn::TrustedReplay,
) -> SqlResult<()> {
    if replay.plies.is_empty() {
        return Ok(());
    }

    let mut sql = String::from(
        "INSERT INTO game_positions (game_id, ply, position_hash, uci_move) VALUES ",
    );
    let mut params: Vec<Box<dyn ToSql>> = Vec::with_capacity(replay.plies.len() * 4);

    for (ply, trusted_ply) in replay.plies.iter().enumerate() {
        if ply > 0 {
            sql.push(',');
        }
        sql.push_str("(?,?,?,?)");

        let hash = hash_to_sql(polyglot_hash(&trusted_ply.position_before));
        let uci  = trusted_ply.mv.to_uci();

        // `ply` is bounded by `OPENING_TREE_MAX_PLIES` (30): no real risk of
        // overflow, the `allow` just documents the intent.
        #[allow(clippy::cast_possible_wrap)]
        let ply_i64 = ply as i64;

        params.push(Box::new(game_id));
        params.push(Box::new(ply_i64));
        params.push(Box::new(hash));
        params.push(Box::new(uci));
    }

    conn.execute(&sql, rusqlite::params_from_iter(params))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Import of a full file
// ---------------------------------------------------------------------------

/// Reads a PGN file, splits the games, and imports them all into the
/// reference database. Equivalent to [`import_pgn_file_with_progress`] with a
/// no-op callback.
///
/// # Errors
///
/// - [`ReferenceImportError::Io`] if the file cannot be read.
/// - [`ReferenceImportError::Sql`] if opening or validating the transaction fails.
pub fn import_pgn_file(conn: &Connection, path: &Path) -> Result<ImportSummary, ReferenceImportError> {
    import_pgn_file_with_progress(conn, path, |_, _| {})
}

/// Identical to [`import_pgn_file`], but calls `on_progress` every
/// [`PROGRESS_EVERY`] games processed, with `(games_processed,
/// file_total)` — to display a progress counter during
/// the import (same principle as
/// [`crate::repository::puzzle_repo::import_csv_with_progress`]).
///
/// # Total known from the very first call (perf bugfix 09/07/2026)
///
/// User feedback: display the number of games in the
/// PGN file from the start, to give a real progress reference rather than a
/// counter that advances without knowing how far it goes. [`split_pgn_games`]
/// already splits **the whole** file into a `Vec<String>` before the import
/// loop starts (it is not a lazy iterator) — the total
/// is therefore `games.len()`, known at zero extra cost (the work was already
/// done), and passed on the very first call to `on_progress` (`(0, total)`),
/// even before the first game is processed.
///
/// `on_progress` is called synchronously, on the same thread as
/// the import itself: it is up to the caller to make it non-blocking if needed
/// (e.g. relaying to the UI via `slint::invoke_from_event_loop` from a
/// dedicated thread).
///
/// # Batched commits (bugfix 09/07/2026)
///
/// Unlike the initial version (a single transaction covering the entire
/// file, like [`crate::import_export::import_pgn_file`]), insertions are
/// now committed (`COMMIT`) every [`COMMIT_EVERY`]
/// games, each batch immediately reopening a new transaction. On a
/// file of several hundred thousand games (e.g. Lumbra's
/// Gigabase), a single transaction would stay open for a very long time and
/// would lose **everything** if interrupted before the final `COMMIT` (app
/// closed, crash, or simply an import too long for the
/// user to wait for completion) — committing in batches preserves the games already
/// processed no matter what happens afterward. A cautious caller can also
/// re-verify the actual game count after the import (`SELECT COUNT(*)
/// FROM games`) rather than trusting only the returned [`ImportSummary`],
/// precisely to detect such a premature stop.
///
/// # Errors
///
/// Same cases as [`import_pgn_file`].
pub fn import_pgn_file_with_progress(
    conn: &Connection,
    path: &Path,
    on_progress: impl FnMut(usize, usize),
) -> Result<ImportSummary, ReferenceImportError> {
    import_pgn_file_with_progress_batched(conn, path, on_progress, COMMIT_EVERY)
}

/// Core of [`import_pgn_file_with_progress`], with `commit_every` as a
/// parameter rather than a fixed constant — only to allow the
/// tests to verify batched-commit behavior on a small
/// number of games without depending on [`COMMIT_EVERY`] (1000, impractical to
/// reach in a unit test).
fn import_pgn_file_with_progress_batched(
    conn: &Connection,
    path: &Path,
    mut on_progress: impl FnMut(usize, usize),
    commit_every: usize,
) -> Result<ImportSummary, ReferenceImportError> {
    let content = std::fs::read_to_string(path)?;
    // `split_pgn_games` already materializes the whole file into a `Vec<String>` —
    // `total` therefore costs nothing more than what was already being done (see
    // the `import_pgn_file_with_progress` doc).
    let games = split_pgn_games(&content);
    let total = games.len();

    let mut summary = ImportSummary::default();
    let mut processed: usize = 0;

    // Immediate visual feedback as soon as reading/splitting the file
    // (potentially long on its own for several hundred MB)
    // is done, even before the first game is processed — avoids
    // a prolonged silence on the UI side at the very start of import, and communicates the
    // total right at that moment (bugfix 09/07/2026).
    on_progress(0, total);

    let mut tx = conn.unchecked_transaction()?;

    for pgn in games {
        processed += 1;

        match import_one(&tx, &pgn) {
            Ok(_id) => summary.imported += 1,
            Err(_)  => summary.skipped += 1,
        }

        if processed.is_multiple_of(PROGRESS_EVERY) {
            on_progress(processed, total);
        }

        if processed.is_multiple_of(commit_every) {
            tx.commit()?;
            tx = conn.unchecked_transaction()?;
        }
    }

    // Final notification: guarantees the caller sees the exact count of
    // games processed even if `processed` is not a multiple of
    // `PROGRESS_EVERY`.
    if !processed.is_multiple_of(PROGRESS_EVERY) {
        on_progress(processed, total);
    }

    tx.commit()?;
    Ok(summary)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::reference_schema::open_in_memory;

    const PGN_MINIMAL: &str = r#"[Event "Test"]
[Site "Local"]
[Date "2024.01.01"]
[Round "1"]
[White "Alice"]
[Black "Bob"]
[Result "1/2-1/2"]

1. e4 e5 1/2-1/2
"#;

    const PGN_ENRICHED: &str = r#"[Event "IRT Test"]
[Site "Lichess"]
[Date "2026.04.06"]
[Round "6"]
[White "Huerfano Rico, Juan Esteban"]
[Black "Ardila Pena, Wilfran"]
[Result "0-1"]
[ECO "A00"]
[WhiteElo "2408"]
[WhiteTitle "IM"]
[BlackElo "2145"]

1. Na3 e5 2. Nb1 d5 3. Nc3 Nc6 0-1
"#;

    const PGN_INVALID: &str = r#"[Event "Bad"]
[White "X"]
[Black "Y"]
[Result "*"]

1. e9 *
"#;

    #[test]
    fn test_import_one_returns_id() {
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_MINIMAL).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_import_one_stores_pgn_verbatim_not_regenerated() {
        // Unlike `import_export::import_pgn_to_db`, the stored text
        // must be EXACTLY the one provided as input (ECO/Elo/Title
        // tags included), not a canonical PGN regenerated from PgnTags (which
        // would lose them).
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();
        let stored: String = conn
            .query_row("SELECT pgn FROM games WHERE id = ?1", [id], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, PGN_ENRICHED);
    }

    #[test]
    fn test_import_one_extracts_enriched_metadata() {
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();
        let (eco, white_elo, white_title, black_elo): (String, i64, String, i64) = conn
            .query_row(
                "SELECT eco, white_elo, white_title, black_elo FROM games WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(eco, "A00");
        assert_eq!(white_elo, 2408);
        assert_eq!(white_title, "IM");
        assert_eq!(black_elo, 2145);
    }

    #[test]
    fn test_import_one_invalid_pgn_fails() {
        let conn = open_in_memory().unwrap();
        let result = import_one(&conn, PGN_INVALID);
        assert!(matches!(result, Err(ReferenceImportError::Pgn(_))));
    }

    #[test]
    fn test_import_one_indexes_opening_positions() {
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();
        // PGN_ENRICHED plays 6 half-moves (3 full moves).
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM game_positions WHERE game_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 6);
    }

    #[test]
    fn test_import_one_first_position_hash_matches_starting_position() {
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();
        let stored_hash: i64 = conn
            .query_row(
                "SELECT position_hash FROM game_positions WHERE game_id = ?1 AND ply = 0",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        let expected = hash_to_sql(polyglot_hash(&Position::starting()));
        assert_eq!(stored_hash, expected);
    }

    #[test]
    fn test_import_one_first_move_stored_as_uci() {
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();
        let uci: String = conn
            .query_row(
                "SELECT uci_move FROM game_positions WHERE game_id = ?1 AND ply = 0",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(uci, "b1a3"); // 1. Na3
    }

    #[test]
    fn test_import_one_indexes_all_plies_with_correct_uci_sequence() {
        // Perf bugfix 09/07/2026: verifies the multi-row write in a
        // single `INSERT` statement in `index_opening_positions` — each
        // row must carry the correct `game_id`, a continuous `ply`
        // sequence (0..N with no gap or duplicate), and the correct
        // `uci_move`, not just the first one (already covered by
        // `test_import_one_first_move_stored_as_uci`).
        let conn = open_in_memory().unwrap();
        let id = import_one(&conn, PGN_ENRICHED).unwrap();

        let mut stmt = conn
            .prepare("SELECT game_id, ply, uci_move FROM game_positions WHERE game_id = ?1 ORDER BY ply")
            .unwrap();
        let rows: Vec<(i64, i64, String)> = stmt
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();

        let expected_uci = ["b1a3", "e7e5", "a3b1", "d7d5", "b1c3", "b8c6"];
        assert_eq!(rows.len(), expected_uci.len());
        for (i, (row_game_id, row_ply, row_uci)) in rows.iter().enumerate() {
            assert_eq!(*row_game_id, id, "ply {i} : game_id incorrect");
            assert_eq!(*row_ply, i64::try_from(i).unwrap(), "ply {i} : numéro de ply incorrect");
            assert_eq!(row_uci, expected_uci[i], "ply {i} : coup UCI incorrect");
        }
    }

    #[test]
    fn test_import_pgn_file_two_games() {
        let conn = open_in_memory().unwrap();
        let two  = format!("{PGN_MINIMAL}\n{PGN_ENRICHED}");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_multi.pgn");
        std::fs::write(&tmp, &two).unwrap();

        let summary = import_pgn_file(&conn, &tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 0);
    }

    #[test]
    fn test_import_pgn_file_skips_invalid_and_counts_it() {
        let conn = open_in_memory().unwrap();
        let mixed = format!("{PGN_MINIMAL}\n{PGN_INVALID}");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_mixed.pgn");
        std::fs::write(&tmp, &mixed).unwrap();

        let summary = import_pgn_file(&conn, &tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(summary.imported, 1);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn test_import_pgn_file_with_progress_reports_immediate_and_final_count() {
        // Bugfix 09/07/2026: an immediate first call (0) right after
        // reading the file, before any game is processed (avoids
        // a prolonged silence on the UI side); 3 games < PROGRESS_EVERY (100), so
        // the exact final count is the only other call. The total (3) is
        // already known and identical from the first call (perf bugfix
        // 09/07/2026).
        let conn = open_in_memory().unwrap();
        let three = format!("{PGN_MINIMAL}\n{PGN_ENRICHED}\n{PGN_MINIMAL}");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_progress.pgn");
        std::fs::write(&tmp, &three).unwrap();

        let mut reports = Vec::new();
        let summary =
            import_pgn_file_with_progress(&conn, &tmp, |done, total| reports.push((done, total))).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(summary.imported, 3);
        assert_eq!(reports, vec![(0, 3), (3, 3)]);
    }

    #[test]
    fn test_import_pgn_file_with_progress_reports_every_100_games() {
        // 250 games, PROGRESS_EVERY = 100 (perf bugfix 09/07/2026, value
        // raised from 25 to 100 following the fast import path): expected
        // reports at 0 (immediate), 100, 200, then 250 (final count, 250
        // not being a multiple of 100) — the total (250) is identical at
        // each call.
        let conn = open_in_memory().unwrap();
        let many = [PGN_MINIMAL; 250].join("\n");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_progress_250.pgn");
        std::fs::write(&tmp, &many).unwrap();

        let mut reports = Vec::new();
        let summary =
            import_pgn_file_with_progress(&conn, &tmp, |done, total| reports.push((done, total))).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(summary.imported, 250);
        assert_eq!(reports, vec![(0, 250), (100, 250), (200, 250), (250, 250)]);
    }

    #[test]
    fn test_import_pgn_file_with_progress_reports_total_immediately() {
        // Response to the user request of 09/07/2026: display the
        // total number of games in the PGN file from the start, to give a
        // real progress reference. `split_pgn_games` already splits the whole
        // file into memory before the import loop — the total is therefore
        // known at zero cost, from the very first call to `on_progress`, before
        // even the first game is processed.
        let conn = open_in_memory().unwrap();
        let five = [PGN_MINIMAL; 5].join("\n");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_total_immediate.pgn");
        std::fs::write(&tmp, &five).unwrap();

        let mut first_call: Option<(usize, usize)> = None;
        let _ = import_pgn_file_with_progress(&conn, &tmp, |done, total| {
            if first_call.is_none() {
                first_call = Some((done, total));
            }
        });
        std::fs::remove_file(&tmp).ok();

        assert_eq!(first_call, Some((0, 5)));
    }

    #[test]
    fn test_import_pgn_file_with_progress_batched_commits_preserve_all_games() {
        // Bugfix 09/07/2026: verifies that batched commits (COMMIT every
        // `commit_every`) do not lose any game compared to a single
        // transaction covering the whole file — here with a batch of 2 out of 5
        // games (3 batches: 2 + 2 + 1), to exercise at least two
        // transaction reopenings without depending on COMMIT_EVERY (1000,
        // impractical in a unit test).
        let conn = open_in_memory().unwrap();
        let five = [PGN_MINIMAL; 5].join("\n");

        let tmp = std::env::temp_dir().join("vendetta_test_reference_batched_commits.pgn");
        std::fs::write(&tmp, &five).unwrap();

        let summary = import_pgn_file_with_progress_batched(&conn, &tmp, |_, _| {}, 2).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(summary.imported, 5);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 5, "les 5 parties doivent être persistées malgré plusieurs COMMIT intermédiaires");
    }

    #[test]
    fn test_index_opening_positions_respects_max_plies_limit() {
        // Fictional 40-half-move game (knights shuffling back and forth,
        // alternating Nf3/Nf6 then Ng1/Ng8 to return to the starting
        // position every two moves — no illegal move, position
        // repetition does not prevent the game from continuing without an
        // explicit draw claim): only the first 30 half-moves should
        // be indexed.
        let conn = open_in_memory().unwrap();
        let mut moves = String::new();
        for k in 1..=20 {
            let (w, b) = if k % 2 == 1 { ("Nf3", "Nf6") } else { ("Ng1", "Ng8") };
            let _ = write!(moves, "{k}. {w} {b} ");
        }
        let pgn = format!(
            "[Event \"Test\"]\n[White \"A\"]\n[Black \"B\"]\n[Result \"*\"]\n\n{moves}*\n"
        );

        let id = import_one(&conn, &pgn).unwrap();
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM game_positions WHERE game_id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count as usize, OPENING_TREE_MAX_PLIES);
    }
}
