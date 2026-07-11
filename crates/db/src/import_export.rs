//! Bridge between the PGN format and the `SQLite` database.
//!
//! ## Import flow
//!
//! ```text
//! PGN (text) → core::pgn::import_pgn → GameState → game_repo::insert → id
//! ```
//!
//! ## Export flow
//!
//! ```text
//! game_id → game_repo::find_by_id → GameRow.pgn → String
//! ```
//!
//! The full PGN is stored as-is in the `pgn` column of the `games`
//! table. Export is therefore a simple read — no reconstruction.

use std::{io, path::Path};

use rusqlite::Connection;

use core::pgn::{self as core_pgn, PgnError, PgnTags};

use crate::repository::game_repo::{self, NewGame};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Error while importing a PGN into the database.
#[derive(Debug)]
pub enum ImportError {
    /// The PGN is invalid or illegal.
    Pgn(PgnError),
    /// `SQLite` error.
    Sql(rusqlite::Error),
    /// File read error.
    Io(io::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pgn(e)  => write!(f, "PGN invalide : {e}"),
            Self::Sql(e)  => write!(f, "Erreur base de données : {e}"),
            Self::Io(e)   => write!(f, "Erreur lecture fichier : {e}"),
        }
    }
}

impl From<PgnError> for ImportError {
    fn from(e: PgnError) -> Self { Self::Pgn(e) }
}

impl From<rusqlite::Error> for ImportError {
    fn from(e: rusqlite::Error) -> Self { Self::Sql(e) }
}

impl From<io::Error> for ImportError {
    fn from(e: io::Error) -> Self { Self::Io(e) }
}

/// Error while exporting a game from the database.
#[derive(Debug)]
pub enum ExportError {
    /// `SQLite` error.
    Sql(rusqlite::Error),
    /// No game found for this identifier.
    NotFound(i64),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(e)        => write!(f, "Erreur base de données : {e}"),
            Self::NotFound(id)  => write!(f, "Partie introuvable (id={id})"),
        }
    }
}

impl From<rusqlite::Error> for ExportError {
    fn from(e: rusqlite::Error) -> Self { Self::Sql(e) }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Parses a PGN, validates the moves, and inserts the game into the database.
///
/// Returns the auto-incremented `id` of the inserted row.
///
/// # Errors
///
/// - [`ImportError::Pgn`] if the PGN is malformed or contains an illegal move.
/// - [`ImportError::Sql`] if the insertion fails.
pub fn import_pgn_to_db(conn: &Connection, pgn: &str) -> Result<i64, ImportError> {
    let game = core_pgn::import_pgn(pgn)?;

    // Extract the tags to fill the dedicated columns.
    let white  = extract_tag(pgn, "White").unwrap_or_else(|| "?".to_string());
    let black  = extract_tag(pgn, "Black").unwrap_or_else(|| "?".to_string());
    let result = extract_tag(pgn, "Result").unwrap_or_else(|| "*".to_string());
    let date   = extract_tag(pgn, "Date");
    let event  = extract_tag(pgn, "Event");
    let site   = extract_tag(pgn, "Site");
    let round  = extract_tag(pgn, "Round");

    // Regenerate a canonical PGN from the validated GameState.
    let tags = PgnTags {
        white:  white.clone(),
        black:  black.clone(),
        result: result.clone(),
        date:   date.clone().unwrap_or_else(|| "????.??.??".to_string()),
        event:  event.clone().unwrap_or_else(|| "?".to_string()),
        site:   site.clone().unwrap_or_else(|| "?".to_string()),
        round:  round.clone().unwrap_or_else(|| "?".to_string()),
    };
    let canonical_pgn = core_pgn::export_pgn(&game, Some(tags));

    let initial_fen = game.initial_fen().to_string();
    let move_count  = i64::try_from(game.move_count()).unwrap_or(i64::MAX);

    let new_game = NewGame {
        tournament_id: None,
        white:         &white,
        black:         &black,
        result:        &result,
        date:          date.as_deref(),
        event:         event.as_deref(),
        site:          site.as_deref(),
        round:         round.as_deref(),
        pgn:           &canonical_pgn,
        initial_fen:   Some(&initial_fen),
        move_count,
    };

    let id = game_repo::insert(conn, &new_game)?;
    Ok(id)
}

/// Returns the PGN stored in the database for game `game_id`.
///
/// # Errors
///
/// - [`ExportError::NotFound`] if the game does not exist.
/// - [`ExportError::Sql`] if the query fails.
pub fn export_pgn_from_db(conn: &Connection, game_id: i64) -> Result<String, ExportError> {
    match game_repo::find_by_id(conn, game_id)? {
        Some(row) => Ok(row.pgn),
        None      => Err(ExportError::NotFound(game_id)),
    }
}

/// Reads a PGN file, splits the games, and inserts them all into the database.
///
/// Returns the list of inserted `id`s. Invalid games are skipped
/// and their errors collected into the output `errors` vector.
///
/// All insertions are grouped into a single `SQLite` transaction
/// (instead of an autocommit per game): for a file with several
/// hundred/thousand games, this avoids an `fsync` per game and significantly
/// reduces import time (perf audit 02/07/2026, point 2).
///
/// # Errors
///
/// - [`ImportError::Io`] if the file cannot be read.
/// - [`ImportError::Sql`] if opening/validating the transaction fails.
pub fn import_pgn_file(
    conn: &Connection,
    path: &Path,
) -> Result<(Vec<i64>, Vec<ImportError>), ImportError> {
    let content = std::fs::read_to_string(path)?;
    let mut ids    = Vec::new();
    let mut errors = Vec::new();

    // `unchecked_transaction` works on `&Connection` (no `&mut` needed):
    // used here because this function's public signature cannot
    // be changed without impacting all existing callers.
    let tx = conn.unchecked_transaction()?;
    for pgn in split_pgn_games(&content) {
        match import_pgn_to_db(&tx, &pgn) {
            Ok(id)  => ids.push(id),
            Err(e)  => errors.push(e),
        }
    }
    tx.commit()?;

    Ok((ids, errors))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Extracts the value of a PGN tag: `[Tag "Value"]` → `Some("Value")`.
///
/// `pub(crate)` (rather than private): reused as-is by
/// `reference_import` (PHASE 82) to extract additional tags
/// (ECO, `WhiteElo`, `BlackElo`, `WhiteTitle`, `BlackTitle`) from the reference games
/// database — avoids duplicating this logic in both modules.
pub(crate) fn extract_tag(pgn: &str, tag: &str) -> Option<String> {
    let prefix = format!("[{tag} \"");
    pgn.lines()
        .find(|l| l.starts_with(&prefix))
        .and_then(|l| {
            let start = prefix.len();
            let end = l[start..].find('"')?;
            Some(l[start..start + end].to_string())
        })
}

/// Splits a multi-game PGN text into individual PGNs.
///
/// The boundary between two games is detected when a tag line `[`
/// appears after a move section (non-tag, non-blank lines).
///
/// `pub(crate)`: reused as-is by `reference_import` (PHASE 82).
pub(crate) fn split_pgn_games(content: &str) -> Vec<String> {
    let mut games   = Vec::new();
    let mut current = String::new();
    let mut in_moves = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // New tag section after moves → new game.
            if in_moves {
                let candidate = current.trim().to_string();
                if !candidate.is_empty() {
                    games.push(candidate);
                }
                current  = String::new();
                in_moves = false;
            }
        } else if !trimmed.is_empty() {
            in_moves = true;
        }
        current.push_str(line);
        current.push('\n');
    }

    let candidate = current.trim().to_string();
    if !candidate.is_empty() {
        games.push(candidate);
    }

    games
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::open_in_memory;

    const PGN_E4_E5: &str = r#"[Event "Test"]
[Site "Local"]
[Date "2024.01.01"]
[Round "1"]
[White "Alice"]
[Black "Bob"]
[Result "1/2-1/2"]

1. e4 e5 1/2-1/2
"#;

    const PGN_SCHOLARS_MATE: &str = r#"[Event "Scholars"]
[Site "Local"]
[Date "2024.01.02"]
[Round "1"]
[White "Carol"]
[Black "Dave"]
[Result "1-0"]

1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# 1-0
"#;

    const PGN_INVALID: &str = r#"[Event "Bad"]
[White "X"]
[Black "Y"]
[Result "*"]

1. e9 *
"#;

    #[test]
    fn test_import_pgn_to_db_returns_id() {
        let conn = open_in_memory().unwrap();
        let id = import_pgn_to_db(&conn, PGN_E4_E5).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_import_stores_metadata() {
        let conn = open_in_memory().unwrap();
        let id = import_pgn_to_db(&conn, PGN_E4_E5).unwrap();
        let row = crate::repository::game_repo::find_by_id(&conn, id)
            .unwrap()
            .unwrap();
        assert_eq!(row.white,  "Alice");
        assert_eq!(row.black,  "Bob");
        assert_eq!(row.result, "1/2-1/2");
        assert_eq!(row.event.as_deref(), Some("Test"));
        assert_eq!(row.move_count, 2);
    }

    #[test]
    fn test_import_stores_pgn() {
        let conn = open_in_memory().unwrap();
        let id = import_pgn_to_db(&conn, PGN_E4_E5).unwrap();
        let row = crate::repository::game_repo::find_by_id(&conn, id)
            .unwrap()
            .unwrap();
        // The stored PGN contains both moves
        assert!(row.pgn.contains("e4"));
        assert!(row.pgn.contains("e5"));
    }

    #[test]
    fn test_import_invalid_pgn_fails() {
        let conn = open_in_memory().unwrap();
        let result = import_pgn_to_db(&conn, PGN_INVALID);
        assert!(matches!(result, Err(ImportError::Pgn(_))));
    }

    #[test]
    fn test_export_pgn_from_db_roundtrip() {
        let conn = open_in_memory().unwrap();
        let id  = import_pgn_to_db(&conn, PGN_SCHOLARS_MATE).unwrap();
        let pgn = export_pgn_from_db(&conn, id).unwrap();
        assert!(pgn.contains("Qxf7#"));
        assert!(pgn.contains("1-0"));
    }

    #[test]
    fn test_export_not_found() {
        let conn = open_in_memory().unwrap();
        let result = export_pgn_from_db(&conn, 9999);
        assert!(matches!(result, Err(ExportError::NotFound(9999))));
    }

    #[test]
    fn test_extract_tag_found() {
        let value = extract_tag(PGN_E4_E5, "White");
        assert_eq!(value, Some("Alice".to_string()));
    }

    #[test]
    fn test_extract_tag_not_found() {
        assert!(extract_tag(PGN_E4_E5, "NonExistent").is_none());
    }

    #[test]
    fn test_split_single_game() {
        let games = split_pgn_games(PGN_E4_E5);
        assert_eq!(games.len(), 1);
    }

    #[test]
    fn test_split_two_games() {
        let two = format!("{PGN_E4_E5}\n{PGN_SCHOLARS_MATE}");
        let games = split_pgn_games(&two);
        assert_eq!(games.len(), 2);
    }

    #[test]
    fn test_import_pgn_file_two_games() {
        let conn = open_in_memory().unwrap();
        let two  = format!("{PGN_E4_E5}\n{PGN_SCHOLARS_MATE}");

        // Write to a temporary file
        let tmp = std::env::temp_dir().join("vendetta_test_multi.pgn");
        std::fs::write(&tmp, &two).unwrap();

        let (ids, errors) = import_pgn_file(&conn, &tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(ids.len(), 2);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_import_pgn_file_skips_invalid() {
        let conn = open_in_memory().unwrap();
        let mixed = format!("{PGN_E4_E5}\n{PGN_INVALID}");

        let tmp = std::env::temp_dir().join("vendetta_test_mixed.pgn");
        std::fs::write(&tmp, &mixed).unwrap();

        let (ids, errors) = import_pgn_file(&conn, &tmp).unwrap();
        std::fs::remove_file(&tmp).ok();

        assert_eq!(ids.len(), 1);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_import_scholars_mate_move_count() {
        let conn = open_in_memory().unwrap();
        let id  = import_pgn_to_db(&conn, PGN_SCHOLARS_MATE).unwrap();
        let row = crate::repository::game_repo::find_by_id(&conn, id)
            .unwrap()
            .unwrap();
        assert_eq!(row.move_count, 7);
    }
}
