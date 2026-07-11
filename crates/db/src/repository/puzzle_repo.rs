//! Repository for tactical puzzles (PHASE 14) and their progress.
//!
//! No puzzle database ships with the software — the user
//! provides their own CSV file (same principle as the Polyglot opening
//! books), typically an export of the Lichess Puzzles database
//! (<https://database.lichess.org/#puzzles>, CC0 license).
//!
//! ## Accepted file formats
//!
//! The official Lichess Puzzles file is distributed compressed
//! (`lichess_db_puzzle.csv.zst`, Zstandard format) and has several
//! million lines. [`import_csv`] therefore accepts two formats, detected
//! by the file extension:
//!
//! - `.csv`: plain text, read directly.
//! - `.zst`: decompressed on the fly (`zstd` crate) before reading — no
//!   manual decompression step is required from the user.
//!
//! In both cases, reading is done in a stream (line by line, via
//! [`std::io::BufRead`]) rather than loading the whole file into memory,
//! to stay reasonable on a file of several hundred MB.
//!
//! ## Expected CSV format
//!
//! Header with at least the columns `PuzzleId`, `FEN`, `Moves`, `Rating`
//! (case-insensitive comparison, free column order). Optional columns
//! recognized if present: `RatingDeviation`, `Popularity`,
//! `NbPlays`, `Themes`, `GameUrl`, `OpeningTags`.
//!
//! The `Moves` column contains the moves in UCI notation separated by
//! spaces: the **first** move brings the position from `FEN` to the position
//! the user must actually solve (opponent move that triggers the
//! puzzle), the following moves alternate solution-move / forced reply.
//!
//! ## `puzzle_progress` — note on column names
//!
//! `puzzle_progress.puzzle_id` references `puzzles.id` (the internal
//! auto-incremented key), **not** `puzzles.puzzle_id` (the external identifier
//! from the source file, e.g. "00008" on Lichess) — foreign key naming
//! convention `<singular table>_id`, like the rest of the project.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};

use core::types::{Move, Position};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A row of the `puzzles` table.
#[derive(Debug, Clone, PartialEq)]
pub struct PuzzleRow {
    pub id:               i64,
    pub puzzle_id:        String,
    pub fen:              String,
    pub moves:            String,
    pub rating:           i64,
    pub rating_deviation: Option<i64>,
    pub popularity:       Option<i64>,
    pub nb_plays:         Option<i64>,
    pub themes:           String,
    pub game_url:         Option<String>,
    pub opening_tags:     Option<String>,
}

/// Result of a puzzle attempt to record in `puzzle_progress`.
///
/// Counting rule validated with the user (03/07/2026): only an
/// attempt where at least one wrong move was played before quitting / viewing
/// the solution / moving to the next one counts. An abandonment with no wrong
/// move attempted is neutral — [`record_attempt`] must then not be called at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptResult {
    Solved,
    Failed,
}

/// Global statistics aggregated over all puzzles attempted at least once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PuzzleStats {
    pub total_attempted: i64,
    pub total_solved:    i64,
}

impl PuzzleStats {
    /// Success rate as a percentage (`0.0` if no attempts).
    ///
    /// Clippy (04/07/2026): `#[allow(cast_precision_loss)]` — these are
    /// puzzle attempt counters, never large enough in practice
    /// to approach the 2^52 threshold beyond which `f64` would lose precision.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn success_rate(&self) -> f64 {
        if self.total_attempted == 0 {
            0.0
        } else {
            (self.total_solved as f64 / self.total_attempted as f64) * 100.0
        }
    }
}

/// Summary of a CSV import: number of puzzles actually inserted and number
/// of lines skipped (invalid format, or duplicate of a `puzzle_id` already
/// present in the database — a reimport of the same file, or of a file that
/// partially overlaps it, therefore never inserts duplicates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped:  usize,
}

// ---------------------------------------------------------------------------
// Import error
// ---------------------------------------------------------------------------

/// Error blocking the **entire** import of a file (unlike
/// individually invalid lines, silently counted in
/// [`ImportSummary::skipped`]).
#[derive(Debug)]
pub enum PuzzleImportError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    /// Empty file (not even a header line).
    EmptyFile,
    /// Invalid CSV header: minimal columns missing.
    MissingColumns(Vec<&'static str>),
}

impl std::fmt::Display for PuzzleImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e)    => write!(f, "Erreur lecture fichier : {e}"),
            Self::Sql(e)   => write!(f, "Erreur base de données : {e}"),
            Self::EmptyFile => write!(f, "Fichier vide"),
            Self::MissingColumns(cols) => write!(
                f,
                "Ce fichier ne ressemble pas à un export de puzzles valide \
                 (colonne(s) manquante(s) : {})",
                cols.join(", ")
            ),
        }
    }
}

impl std::error::Error for PuzzleImportError {}

impl From<std::io::Error> for PuzzleImportError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

impl From<rusqlite::Error> for PuzzleImportError {
    fn from(e: rusqlite::Error) -> Self { Self::Sql(e) }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

const REQUIRED_COLUMNS: [&str; 4] = ["puzzleid", "fen", "moves", "rating"];

/// Frequency (in number of data lines processed, header excluded) at
/// which [`import_csv_with_progress`] notifies its progress callback.
/// Chosen to stay inexpensive on a file of several million
/// lines (e.g. the full Lichess Puzzles export, ~300 MB) while giving
/// frequent visual feedback to the user (PHASE 14, user feedback of
/// 03/07/2026: a long import with no indicator looks like a crash).
const PROGRESS_EVERY: usize = 2000;

/// Opens a puzzle file for reading, with transparent Zstandard
/// decompression if the extension is `.zst` (the case for the official
/// `lichess_db_puzzle.csv.zst` file). Case-insensitive extension
/// comparison. Any other extension (typically `.csv`) is read as-is.
///
/// Returns a boxed `BufRead` to let [`import_csv`] handle
/// both cases uniformly, as a stream (line by line).
fn open_puzzle_reader(path: &Path) -> Result<Box<dyn BufRead>, PuzzleImportError> {
    let file = File::open(path)?;
    let is_zst = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zst"));

    if is_zst {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Imports a puzzle file (e.g. Lichess Puzzles export) into the database.
///
/// Accepts both a plain-text CSV (`.csv`) and the official
/// Zstandard-compressed file (`.zst`) — see the module note above. The
/// detection is based solely on the file extension; the decompressed
/// content must be a valid CSV in both cases.
///
/// First checks the header (minimal columns `PuzzleId`/`FEN`/`Moves`/
/// `Rating`, case-insensitive comparison, free order) — rejects the
/// whole file if the header does not match at all, before even
/// starting the import. Then, each line is validated individually
/// (field count consistent with the header, parsable FEN, well-formed
/// UCI moves, numeric rating): an invalid line is **skipped and
/// counted**, without blocking the import of the rest of the file — same principle as
/// [`crate::import_export::import_pgn_file`] for invalid PGN games.
///
/// Reading is done in a stream (line by line) rather than loading
/// the whole file into memory at once, and all insertions are grouped
/// into a single `SQLite` transaction (Lichess files can have
/// several million lines).
///
/// # Errors
///
/// - [`PuzzleImportError::Io`] if the file cannot be read, or if the
///   `.zst` stream is corrupted / malformed.
/// - [`PuzzleImportError::EmptyFile`] if the file does not even contain a header.
/// - [`PuzzleImportError::MissingColumns`] if the header does not contain the
///   expected minimal columns.
/// - [`PuzzleImportError::Sql`] if opening or validating the transaction fails.
///
/// Reports no progress — equivalent to
/// [`import_csv_with_progress`] with a no-op callback. Kept
/// as-is (unchanged signature) so as not to disrupt existing
/// callers that don't need to track progress (e.g. tests).
pub fn import_csv(conn: &Connection, path: &Path) -> Result<ImportSummary, PuzzleImportError> {
    import_csv_with_progress(conn, path, |_| {})
}

/// Identical to [`import_csv`], but calls `on_progress` every
/// [`PROGRESS_EVERY`] data lines processed (header excluded), with the
/// cumulative number of lines read so far — to display a progress
/// indicator during the import of a large file (PHASE 14, user
/// feedback of 03/07/2026: a 300 MB file can take long enough
/// to look like a crash without visual feedback).
///
/// `on_progress` is called synchronously, on the same thread as
/// the import itself: it is up to the caller to make it non-blocking if needed
/// (e.g. relaying to the UI via `slint::invoke_from_event_loop` from a
/// dedicated thread, rather than calling this function directly on the
/// UI thread).
///
/// # Errors
///
/// Same cases as [`import_csv`].
///
/// # Panics
/// Does not panic in practice: the internal `.unwrap()`s on `col_index(...)`
/// concern columns whose presence in the header has just been
/// verified right above (`REQUIRED_COLUMNS`).
pub fn import_csv_with_progress(
    conn: &Connection,
    path: &Path,
    mut on_progress: impl FnMut(usize),
) -> Result<ImportSummary, PuzzleImportError> {
    let reader = open_puzzle_reader(path)?;
    let mut lines = reader.lines();

    let header_line = lines.next().ok_or(PuzzleImportError::EmptyFile)??;
    let header: Vec<String> = header_line
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .collect();

    let missing: Vec<&'static str> = REQUIRED_COLUMNS
        .iter()
        .filter(|col| !header.iter().any(|h| h == *col))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(PuzzleImportError::MissingColumns(missing));
    }

    let col_index = |name: &str| header.iter().position(|h| h == name);
    // Safe `.unwrap()`s: presence already guaranteed by the check above.
    let idx_puzzle_id     = col_index("puzzleid").unwrap();
    let idx_fen           = col_index("fen").unwrap();
    let idx_moves         = col_index("moves").unwrap();
    let idx_rating        = col_index("rating").unwrap();
    let idx_rating_dev    = col_index("ratingdeviation");
    let idx_popularity    = col_index("popularity");
    let idx_nb_plays      = col_index("nbplays");
    let idx_themes        = col_index("themes");
    let idx_game_url      = col_index("gameurl");
    let idx_opening_tags  = col_index("openingtags");

    let mut summary = ImportSummary::default();
    let tx = conn.unchecked_transaction()?;
    let mut processed: usize = 0;

    for line in lines {
        // An error here (invalid UTF-8, corrupted `.zst` stream) is fatal:
        // unlike a structurally invalid CSV line (counted
        // in `skipped`), it indicates a file unreadable in its
        // entirety, not just a single line to skip.
        let line = line?;
        processed += 1;

        // Notified every `PROGRESS_EVERY` lines processed, whether they
        // are imported or skipped, to stay representative of
        // actual progress through the file.
        if processed.is_multiple_of(PROGRESS_EVERY) {
            on_progress(processed);
        }

        if line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() != header.len() {
            summary.skipped += 1;
            continue;
        }

        let puzzle_id = fields[idx_puzzle_id].trim();
        let fen       = fields[idx_fen].trim();
        let moves     = fields[idx_moves].trim();

        let Ok(rating) = fields[idx_rating].trim().parse::<i64>() else {
            summary.skipped += 1;
            continue;
        };

        if puzzle_id.is_empty() || !is_valid_puzzle_row(fen, moves) {
            summary.skipped += 1;
            continue;
        }

        let rating_deviation = idx_rating_dev.and_then(|i| fields[i].trim().parse::<i64>().ok());
        let popularity       = idx_popularity.and_then(|i| fields[i].trim().parse::<i64>().ok());
        let nb_plays         = idx_nb_plays.and_then(|i| fields[i].trim().parse::<i64>().ok());
        let themes           = idx_themes.map(|i| fields[i].trim().to_string()).unwrap_or_default();
        let game_url         = idx_game_url
            .map(|i| fields[i].trim().to_string())
            .filter(|s| !s.is_empty());
        let opening_tags     = idx_opening_tags
            .map(|i| fields[i].trim().to_string())
            .filter(|s| !s.is_empty());

        // `INSERT OR IGNORE`: a `puzzle_id` already present (reimport of an
        // identical or overlapping file) is silently ignored
        // rather than failing the whole transaction on the UNIQUE
        // constraint.
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO puzzles
                 (puzzle_id, fen, moves, rating, rating_deviation, popularity,
                  nb_plays, themes, game_url, opening_tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                puzzle_id, fen, moves, rating, rating_deviation, popularity,
                nb_plays, themes, game_url, opening_tags,
            ],
        )?;

        if inserted > 0 {
            summary.imported += 1;
        } else {
            summary.skipped += 1;
        }
    }

    // Final notification: guarantees the caller sees the exact count of
    // lines processed even if `processed` is not a multiple of
    // `PROGRESS_EVERY` (file whose last chunk is incomplete).
    if !processed.is_multiple_of(PROGRESS_EVERY) {
        on_progress(processed);
    }

    tx.commit()?;
    Ok(summary)
}

/// Fully empties the puzzle database: progress statistics
/// (`puzzle_progress`) then the puzzles themselves (`puzzles`).
///
/// Explicitly deletes from both tables rather than relying on the
/// `ON DELETE CASCADE` constraint of `puzzle_progress.puzzle_id` (PHASE 14,
/// 03/07/2026).
///
/// Perf fix of 05/07/2026 (user report: the "Unload" button
/// in Preferences was very slow): contrary to what this function's old
/// comment said, `open_and_migrate` does indeed enable
/// `PRAGMA foreign_keys=ON` (see `configure_pragmas` in `schema.rs`).
/// Now `SQLite` automatically disables its "truncate optimization" — which
/// normally makes a `DELETE FROM table` with no `WHERE` clause nearly instant by
/// freeing the btree pages in bulk rather than visiting each row —
/// as soon as a table is referenced by an active foreign key constraint.
/// This is the case for `puzzles`, referenced by
/// `puzzle_progress.puzzle_id ... ON DELETE CASCADE`: the second `DELETE
/// FROM` was therefore doing a real row-by-row scan, very noticeable on a
/// Lichess database of several hundred thousand puzzles.
///
/// `PRAGMA foreign_keys` is therefore temporarily disabled (necessarily
/// **outside** a transaction: `SQLite` refuses this `PRAGMA` inside an
/// open transaction) for the duration of the clear, which restores the truncate
/// optimization for both `DELETE FROM`s, then it is systematically
/// re-enabled afterward — even if the clear failed — so as to never
/// leave the connection durably without referential integrity
/// constraints (import, individual puzzle deletion, etc. for the
/// rest of the session).
///
/// # Errors
///
/// Returns a `SQLite` error if one of the deletions fails, or if
/// re-enabling `PRAGMA foreign_keys=ON` fails after a successful clear.
pub fn clear_all(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")?;

    let result = clear_all_inner(conn);

    // Systematic re-enabling, even in case of the error above.
    let reenable = conn.execute_batch("PRAGMA foreign_keys=ON;");

    result.and(reenable)
}

/// Transactional body of [`clear_all`], executed with `PRAGMA
/// foreign_keys=OFF` already in place.
fn clear_all_inner(conn: &Connection) -> SqlResult<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM puzzle_progress", [])?;
    tx.execute("DELETE FROM puzzles", [])?;
    tx.commit()?;
    Ok(())
}

/// Checks that a puzzle row is structurally valid: parsable FEN
/// and non-empty UCI move sequence where each move is well-formed.
///
/// Deliberately does not check the full legality of the sequence
/// (replaying each move on the board) — a disproportionate cost at
/// import time for files that can have several million lines
/// from an already-trusted source (Lichess). A puzzle move that is
/// actually unplayable will in any case be detected without crashing at
/// resolution time (Step 4), as for Polyglot book moves
/// (`apply_uci_move_from_book` returns `false` without panicking).
fn is_valid_puzzle_row(fen: &str, moves: &str) -> bool {
    if Position::from_fen(fen).is_err() {
        return false;
    }
    if moves.is_empty() {
        return false;
    }
    moves.split_whitespace().all(|mv| Move::from_uci(mv).is_some())
}

// ---------------------------------------------------------------------------
// Selection and progress
// ---------------------------------------------------------------------------

/// Draws a random puzzle from the whole database (pure random, including
/// already-solved puzzles — V1 decision of 03/07/2026). Returns `None` if the
/// database is empty.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn random_puzzle(conn: &Connection) -> SqlResult<Option<PuzzleRow>> {
    conn.query_row(
        "SELECT id, puzzle_id, fen, moves, rating, rating_deviation, popularity,
                nb_plays, themes, game_url, opening_tags
         FROM puzzles ORDER BY RANDOM() LIMIT 1",
        [],
        row_to_puzzle,
    )
    .optional()
}

/// Looks up a puzzle by its internal `id`. Returns `None` if not found.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn find_by_id(conn: &Connection, id: i64) -> SqlResult<Option<PuzzleRow>> {
    conn.query_row(
        "SELECT id, puzzle_id, fen, moves, rating, rating_deviation, popularity,
                nb_plays, themes, game_url, opening_tags
         FROM puzzles WHERE id = ?1",
        [id],
        row_to_puzzle,
    )
    .optional()
}

/// Total number of puzzles in the database.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn count(conn: &Connection) -> SqlResult<i64> {
    conn.query_row("SELECT COUNT(*) FROM puzzles", [], |row| row.get(0))
}

/// Records the result of a puzzle attempt in `puzzle_progress`.
///
/// **Must only be called if the user actually attempted at
/// least one wrong move, or solved the puzzle** — an abandonment with no
/// wrong-move attempt is neutral and must *not* call this function
/// (counting rule validated with the user, PHASE 14).
///
/// Creates the `puzzle_progress` row on the first call for this puzzle, then
/// updates it (`times_attempted` incremented on each call, `times_solved`
/// only on [`AttemptResult::Solved`]).
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails (e.g. `puzzle_id`
/// nonexistent in `puzzles`).
pub fn record_attempt(conn: &Connection, puzzle_id: i64, result: AttemptResult) -> SqlResult<()> {
    let (solved_inc, last_result) = match result {
        AttemptResult::Solved => (1i64, "solved"),
        AttemptResult::Failed => (0i64, "failed"),
    };
    conn.execute(
        "INSERT INTO puzzle_progress (puzzle_id, times_attempted, times_solved, last_result, last_seen_at)
         VALUES (?1, 1, ?2, ?3, datetime('now'))
         ON CONFLICT(puzzle_id) DO UPDATE SET
             times_attempted = times_attempted + 1,
             times_solved    = times_solved + ?2,
             last_result     = ?3,
             last_seen_at    = datetime('now')",
        params![puzzle_id, solved_inc, last_result],
    )?;
    Ok(())
}

/// Global statistics aggregated over all puzzles attempted at least once.
///
/// # Errors
///
/// Returns a `SQLite` error if the query fails.
pub fn global_stats(conn: &Connection) -> SqlResult<PuzzleStats> {
    conn.query_row(
        "SELECT COALESCE(SUM(times_attempted), 0), COALESCE(SUM(times_solved), 0)
         FROM puzzle_progress",
        [],
        |row| {
            Ok(PuzzleStats {
                total_attempted: row.get(0)?,
                total_solved:    row.get(1)?,
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn row_to_puzzle(row: &rusqlite::Row<'_>) -> rusqlite::Result<PuzzleRow> {
    Ok(PuzzleRow {
        id:               row.get(0)?,
        puzzle_id:        row.get(1)?,
        fen:              row.get(2)?,
        moves:            row.get(3)?,
        rating:           row.get(4)?,
        rating_deviation: row.get(5)?,
        popularity:       row.get(6)?,
        nb_plays:         row.get(7)?,
        themes:           row.get(8)?,
        game_url:         row.get(9)?,
        opening_tags:     row.get(10)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::open_in_memory;
    use std::fmt::Write as _;

    const VALID_CSV: &str = "\
PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags
csvtest001,r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3,e8g8 f3e5 c6e5 c4f7,1450,80,90,120,fork middlegame,https://lichess.org/abc,Italian_Game
csvtest002,rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1,e2e4 e7e5,1000,90,75,50,opening,,
";

    /// Writes `content` compressed with Zstandard, to test the `.zst` branch
    /// of [`open_puzzle_reader`] without depending on an external file.
    fn write_tmp_csv_zst(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let compressed = zstd::stream::encode_all(content.as_bytes(), 0).unwrap();
        std::fs::write(&path, compressed).unwrap();
        path
    }

    fn write_tmp_csv(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_import_csv_valid_file() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_valid.csv", VALID_CSV);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 0);
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn test_import_csv_zst_valid_file() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv_zst("vendetta_test_puzzles_valid.csv.zst", VALID_CSV);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 0);
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn test_import_csv_zst_extension_case_insensitive() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv_zst("vendetta_test_puzzles_valid_upper.csv.ZST", VALID_CSV);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 2);
    }

    #[test]
    fn test_import_csv_rejects_missing_columns() {
        let conn = open_in_memory().unwrap();
        let bad = "Id,Position,Solution\n1,x,y\n";
        let path = write_tmp_csv("vendetta_test_puzzles_bad_header.csv", bad);
        let result = import_csv(&conn, &path);
        std::fs::remove_file(&path).ok();

        assert!(matches!(result, Err(PuzzleImportError::MissingColumns(_))));
    }

    #[test]
    fn test_import_csv_empty_file() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_empty.csv", "");
        let result = import_csv(&conn, &path);
        std::fs::remove_file(&path).ok();

        assert!(matches!(result, Err(PuzzleImportError::EmptyFile)));
    }

    #[test]
    fn test_import_csv_skips_invalid_fen() {
        let conn = open_in_memory().unwrap();
        let mixed = format!(
            "{VALID_CSV}badfen001,not-a-valid-fen,e2e4,1200,,,,,,\n"
        );
        let path = write_tmp_csv("vendetta_test_puzzles_bad_fen.csv", &mixed);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn test_import_csv_skips_malformed_moves() {
        let conn = open_in_memory().unwrap();
        let mixed = format!(
            "{VALID_CSV}badmoves001,rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1,not-a-move,1200,,,,,,\n"
        );
        let path = write_tmp_csv("vendetta_test_puzzles_bad_moves.csv", &mixed);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 2);
        assert_eq!(summary.skipped, 1);
    }

    #[test]
    fn test_import_csv_skips_duplicate_puzzle_id() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_dup.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        // Reimport of the same file: everything must be skipped (duplicates), nothing added.
        let summary2 = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary2.imported, 0);
        assert_eq!(summary2.skipped, 2);
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn test_import_csv_column_order_independent() {
        let conn = open_in_memory().unwrap();
        // Header in a different order, different case.
        let reordered = "\
rating,fen,moves,puzzleid\n\
900,rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1,e2e4 e7e5,reordered001\n";
        let path = write_tmp_csv("vendetta_test_puzzles_reordered.csv", reordered);
        let summary = import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, 1);
        let row = random_puzzle(&conn).unwrap().unwrap();
        assert_eq!(row.puzzle_id, "reordered001");
        assert_eq!(row.rating, 900);
    }

    #[test]
    fn test_random_puzzle_empty_db() {
        let conn = open_in_memory().unwrap();
        assert!(random_puzzle(&conn).unwrap().is_none());
    }

    #[test]
    fn test_random_puzzle_returns_row() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_random.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        let row = random_puzzle(&conn).unwrap().unwrap();
        assert!(row.id > 0);
        assert!(!row.fen.is_empty());
    }

    #[test]
    fn test_find_by_id_found_and_not_found() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_findid.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();

        let row = random_puzzle(&conn).unwrap().unwrap();
        let found = find_by_id(&conn, row.id).unwrap().unwrap();
        assert_eq!(found.puzzle_id, row.puzzle_id);
        assert!(find_by_id(&conn, 999_999).unwrap().is_none());
    }

    #[test]
    fn test_record_attempt_creates_progress_row() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_attempt1.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();
        let row = random_puzzle(&conn).unwrap().unwrap();

        record_attempt(&conn, row.id, AttemptResult::Failed).unwrap();

        let stats = global_stats(&conn).unwrap();
        assert_eq!(stats.total_attempted, 1);
        assert_eq!(stats.total_solved, 0);
    }

    #[test]
    fn test_record_attempt_accumulates() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_attempt2.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();
        let row = random_puzzle(&conn).unwrap().unwrap();

        record_attempt(&conn, row.id, AttemptResult::Failed).unwrap();
        record_attempt(&conn, row.id, AttemptResult::Solved).unwrap();

        let stats = global_stats(&conn).unwrap();
        assert_eq!(stats.total_attempted, 2);
        assert_eq!(stats.total_solved, 1);
        assert!((stats.success_rate() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    // Clippy (04/07/2026): `#[allow(float_cmp)]` — `success_rate()` returns
    // exactly `0.0` (early return on `total_attempted == 0`, not a
    // rounded floating-point computation); the strict comparison is intentional.
    #[allow(clippy::float_cmp)]
    fn test_global_stats_empty() {
        let conn = open_in_memory().unwrap();
        let stats = global_stats(&conn).unwrap();
        assert_eq!(stats.total_attempted, 0);
        assert_eq!(stats.total_solved, 0);
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_record_attempt_unknown_puzzle_fails() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let result = record_attempt(&conn, 999_999, AttemptResult::Failed);
        assert!(result.is_err());
    }

    /// Generates a valid CSV of `n` data lines (beyond the two lines
    /// of `VALID_CSV`), to exceed the `PROGRESS_EVERY` threshold several times
    /// and verify that the progress callback is indeed called several
    /// times with increasing values.
    fn write_tmp_large_csv(name: &str, n: usize) -> std::path::PathBuf {
        let mut content = String::from(
            "PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,GameUrl,OpeningTags\n",
        );
        for i in 0..n {
            // write! on a String cannot fail (cf. std impl); the
            // Result is intentionally ignored (clippy::format_push_string).
            let _ = writeln!(
                content,
                "gen{i:06},rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1,e2e4 e7e5,1000,,,,,,"
            );
        }
        write_tmp_csv(name, &content)
    }

    #[test]
    fn test_import_csv_with_progress_reports_progress() {
        let conn = open_in_memory().unwrap();
        // 3 times PROGRESS_EVERY plus a remainder, to check both the
        // intermediate notifications and the final notification.
        let n = PROGRESS_EVERY * 3 + 500;
        let path = write_tmp_large_csv("vendetta_test_puzzles_progress.csv", n);

        let mut reports: Vec<usize> = Vec::new();
        let summary =
            import_csv_with_progress(&conn, &path, |done| reports.push(done)).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(summary.imported, n);
        // Three intermediate steps plus a final report for the remainder.
        assert_eq!(reports.len(), 4);
        assert_eq!(reports[0], PROGRESS_EVERY);
        assert_eq!(reports[1], PROGRESS_EVERY * 2);
        assert_eq!(reports[2], PROGRESS_EVERY * 3);
        assert_eq!(reports[3], n);
        // Strictly increasing.
        assert!(reports.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn test_import_csv_with_progress_no_final_report_on_exact_multiple() {
        let conn = open_in_memory().unwrap();
        let n = PROGRESS_EVERY * 2;
        let path = write_tmp_large_csv("vendetta_test_puzzles_progress_exact.csv", n);

        let mut reports: Vec<usize> = Vec::new();
        import_csv_with_progress(&conn, &path, |done| reports.push(done)).unwrap();
        std::fs::remove_file(&path).ok();

        // No redundant final report when `n` lands exactly on a step.
        assert_eq!(reports, vec![PROGRESS_EVERY, PROGRESS_EVERY * 2]);
    }

    #[test]
    fn test_clear_all_empties_puzzles_and_progress() {
        let conn = open_in_memory().unwrap();
        let path = write_tmp_csv("vendetta_test_puzzles_clear_all.csv", VALID_CSV);
        import_csv(&conn, &path).unwrap();
        std::fs::remove_file(&path).ok();
        let row = random_puzzle(&conn).unwrap().unwrap();
        record_attempt(&conn, row.id, AttemptResult::Solved).unwrap();

        assert_eq!(count(&conn).unwrap(), 2);
        assert_eq!(global_stats(&conn).unwrap().total_attempted, 1);

        clear_all(&conn).unwrap();

        assert_eq!(count(&conn).unwrap(), 0);
        let stats = global_stats(&conn).unwrap();
        assert_eq!(stats.total_attempted, 0);
        assert_eq!(stats.total_solved, 0);
        assert!(random_puzzle(&conn).unwrap().is_none());
    }

    #[test]
    fn test_clear_all_on_empty_db_is_noop() {
        let conn = open_in_memory().unwrap();
        clear_all(&conn).unwrap();
        assert_eq!(count(&conn).unwrap(), 0);
    }

    // Perf fix of 05/07/2026: `clear_all` temporarily disables
    // `PRAGMA foreign_keys` (to regain SQLite's truncate optimization
    // on DELETE FROM without WHERE) then must re-enable it before
    // returning control — otherwise the rest of the session would run without
    // referential integrity constraints.
    #[test]
    fn test_clear_all_restores_foreign_keys_pragma() {
        let conn = open_in_memory().unwrap();
        clear_all(&conn).unwrap();
        let fk_enabled: bool = conn
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .unwrap();
        assert!(fk_enabled, "PRAGMA foreign_keys doit rester actif après clear_all");
    }
}
