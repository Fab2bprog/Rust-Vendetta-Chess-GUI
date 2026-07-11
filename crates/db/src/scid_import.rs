//! Import of SCID databases (si4: `.si4`/`.sn4`/`.sg4`; si5: `.si5`/`.sn5`/
//! `.sg5`, since V2 Phase C1, 12/07/2026, task #21) into the
//! reference games database.
//!
//! Architectural principle (see `SUIVI_PLAN_ACTION.md`, "import
//! SCID" discussion): the `scid` crate fully decodes the SCID binary and produces
//! PGN text; this module simply calls
//! [`reference_import::import_one`] — UNCHANGED — game by game, following
//! the exact model of [`reference_import::import_pgn_file`]. This module
//! knows nothing about the SCID binary format itself.
//!
//! Error policy: the failure of a SINGLE game (variation encountered,
//! non-standard starting position, illegal move once resolved...) does not
//! fail the whole import — it is counted in
//! [`reference_import::ImportSummary::skipped`], exactly like an individually
//! invalid PGN in a multi-game PGN file. Only the
//! inability to OPEN the database (missing file, invalid magic/version) is a
//! fatal error for the entire import.
//!
//! si4 and si5 share a single import loop ([`import_scid_source`])
//! via the local [`ScidSource`] trait, implemented for `scid::Si4Database` and
//! `scid::Si5Database` — avoids duplicating (and letting silently
//! diverge) the progress/batch/diagnostic logic between the two
//! formats, which only differ in how the files are opened.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use rusqlite::Connection;

use crate::reference_import::{self, ImportSummary, ReferenceImportError, COMMIT_EVERY, PROGRESS_EVERY};

/// Local abstraction over an already-open SCID database (si4 OR si5) — only
/// the opening (`Si4Database::open`/`Si5Database::open`, very different
/// disk formats) distinguishes the two; the rest of the import loop is
/// entirely shared via this trait.
trait ScidSource {
    fn game_count(&self) -> usize;
    fn game_pgn(&self, n: usize) -> Result<String, scid::GameDecodeError>;
}

impl ScidSource for scid::Si4Database {
    fn game_count(&self) -> usize { Self::game_count(self) }
    fn game_pgn(&self, n: usize) -> Result<String, scid::GameDecodeError> { Self::game_pgn(self, n) }
}

impl ScidSource for scid::Si5Database {
    fn game_count(&self) -> usize { Self::game_count(self) }
    fn game_pgn(&self, n: usize) -> Result<String, scid::GameDecodeError> { Self::game_pgn(self, n) }
}

/// Diagnostic label for a skipped game — used only for
/// end-of-import counting/logging (12/07/2026, user request: "just
/// a display/log, nothing intrusive"), does not affect the result or the
/// behavior of the import. Distinguishes accepted V1 limitations
/// (variations, non-standard position...) from signals that would rather
/// point to an actual decoding bug or a corrupted file (illegal move,
/// corrupted stream, out-of-bounds offset).
fn scid_decode_error_label(e: &scid::GameDecodeError) -> &'static str {
    use scid::GameDecodeError as E;
    match e {
        // Never returned anymore by `scid::game_blob` since V2 Phase C2
        // (12/07/2026, task #22 — non-standard starting positions
        // now decoded); kept for match exhaustiveness.
        E::NonStandardStart   => "position de départ non standard (cas normalement impossible depuis Phase C2)",
        // Never returned anymore since V2 Phase D (13/07/2026, task #23 —
        // variations now decoded); kept for exhaustiveness.
        E::ContainsVariations => "partie avec variantes (cas normalement impossible depuis Phase D)",
        E::NullMove           => "coup nul rencontré",
        E::BadMoveStream(_)   => "flux de coups corrompu (bug de décodage possible)",
        E::IllegalMove { .. } => "coup illégal détecté au décodage (bug de décodage possible)",
        E::BadOffset          => "offset/longueur hors bornes du fichier .sg4 (fichier corrompu ?)",
    }
}

/// Same principle as [`scid_decode_error_label`], for errors from the
/// second pass ([`reference_import::import_one`], which reparses the generated PGN).
fn reference_import_error_label(e: &ReferenceImportError) -> &'static str {
    match e {
        ReferenceImportError::Pgn(_) => "PGN généré rejeté à la revalidation (bug de décodage/export probable)",
        ReferenceImportError::Sql(_) => "erreur SQL à l'insertion",
        ReferenceImportError::Io(_)  => "erreur d'E/S à l'insertion",
    }
}

/// Imports all decodable games from a si4 database (`.si4`/`.sn4`/`.sg4`,
/// the last two files being derived from the `.si4` path) into the
/// reference database.
///
/// # Errors
/// [`ReferenceImportError::Io`] if the si4 database itself cannot be
/// opened (missing file, invalid magic/version, truncated file) —
/// a fatal error, unlike a single game's decode failure
/// (see the error policy at the top of this module).
pub fn import_si4_file(conn: &Connection, si4_path: &Path) -> Result<ImportSummary, ReferenceImportError> {
    import_si4_file_with_progress(conn, si4_path, |_, _| {})
}

/// Same as [`import_si4_file`], with progress tracking — see
/// [`import_scid_source`] for the full detail of the behavior (shared
/// with [`import_si5_file_with_progress`]).
///
/// # Errors
/// See [`import_si4_file`].
pub fn import_si4_file_with_progress(
    conn: &Connection,
    si4_path: &Path,
    on_progress: impl FnMut(usize, usize),
) -> Result<ImportSummary, ReferenceImportError> {
    let paths = scid::Si4Paths::from_index_path(si4_path);
    let db = scid::Si4Database::open(&paths).map_err(|e| {
        ReferenceImportError::Io(io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    })?;
    import_scid_source(conn, &db, on_progress)
}

/// Imports all decodable games from a si5 database (`.si5`/`.sn5`/`.sg5`,
/// the last two files being derived from the `.si5` path) into the
/// reference database — an exact mirror of [`import_si4_file`] (V2 Phase C1,
/// 12/07/2026, task #21).
///
/// # Errors
/// See [`import_si4_file`] (same error policy).
pub fn import_si5_file(conn: &Connection, si5_path: &Path) -> Result<ImportSummary, ReferenceImportError> {
    import_si5_file_with_progress(conn, si5_path, |_, _| {})
}

/// Same as [`import_si5_file`], with progress tracking — see
/// [`import_scid_source`] for the full detail of the behavior (shared
/// with [`import_si4_file_with_progress`]).
///
/// # Errors
/// See [`import_si5_file`].
pub fn import_si5_file_with_progress(
    conn: &Connection,
    si5_path: &Path,
    on_progress: impl FnMut(usize, usize),
) -> Result<ImportSummary, ReferenceImportError> {
    let paths = scid::Si5Paths::from_index_path(si5_path);
    let db = scid::Si5Database::open(&paths).map_err(|e| {
        ReferenceImportError::Io(io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    })?;
    import_scid_source(conn, &db, on_progress)
}

/// Shared core of the SCID import (si4 and si5, via the [`ScidSource`] trait):
/// calls `on_progress` every [`PROGRESS_EVERY`] games processed (plus
/// a final call guaranteeing the exact count if `total` is not a
/// multiple of `PROGRESS_EVERY`), with `(games_processed, database_total)`
/// — the total is known as soon as the database is opened, so it is passed
/// on the very first call, `(0, total)`, before even the first game — same
/// principle (and the same constant) as
/// [`reference_import::import_pgn_file_with_progress`]
/// (bugfix 12/07/2026, user feedback: one call per game made the
/// counter scroll too fast to be readable).
///
/// The whole database is imported — the temporary 500-game test cap that
/// existed during the development/testing phase was lifted on explicit
/// user request (11/07/2026); see `SUIVI_PLAN_ACTION.md`.
///
/// `on_progress` is called synchronously, on the same thread as
/// the import itself: it is up to the caller to make it non-blocking if needed.
///
/// # Performance (V2 Phase A1, 12/07/2026, task #18)
///
/// Insertions are grouped into transactions in batches of
/// [`COMMIT_EVERY`] games, exactly like
/// [`reference_import::import_pgn_file_with_progress`] (same constant,
/// shared) — replaces `rusqlite`'s default autocommit mode (an
/// implicit `COMMIT` per game), noticeably slower on a large database.
/// If interrupted partway through, only the current batch (up to
/// `COMMIT_EVERY` games) is lost, not the entire import — the same
/// tradeoff as for PGN.
///
/// # Errors
/// [`ReferenceImportError::Sql`] if opening/validating a transaction
/// fails — a fatal error for the entire import. Decode errors for a
/// SINGLE game never propagate up here (see the error policy
/// at the top of this module).
fn import_scid_source(
    conn: &Connection,
    db: &impl ScidSource,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<ImportSummary, ReferenceImportError> {
    let total = db.game_count();
    on_progress(0, total);

    // Diagnostic count by error type (12/07/2026): does not influence
    // `summary` or the behavior of the import, only displayed at the end of the
    // function via `eprintln!` (see the function). Key = label returned
    // by `scid_decode_error_label`/`reference_import_error_label`.
    let mut reasons: HashMap<&'static str, usize> = HashMap::new();

    let mut summary = ImportSummary::default();
    let mut processed: usize = 0;
    let mut tx = conn.unchecked_transaction()?;
    for n in 0..total {
        let imported_ok = match db.game_pgn(n) {
            Ok(pgn) => match reference_import::import_one(&tx, &pgn) {
                Ok(_id) => true,
                Err(e) => {
                    *reasons.entry(reference_import_error_label(&e)).or_insert(0) += 1;
                    false
                }
            },
            Err(e) => {
                *reasons.entry(scid_decode_error_label(&e)).or_insert(0) += 1;
                false
            }
        };
        if imported_ok {
            summary.imported += 1;
        } else {
            summary.skipped += 1;
        }

        processed += 1;
        if processed.is_multiple_of(PROGRESS_EVERY) {
            on_progress(processed, total);
        }
        if processed.is_multiple_of(COMMIT_EVERY) {
            tx.commit()?;
            tx = conn.unchecked_transaction()?;
        }
    }

    // Final notification: guarantees the caller sees the exact count of
    // games processed even if `processed` is not a multiple of
    // `PROGRESS_EVERY` — same principle as
    // `reference_import::import_pgn_file_with_progress_batched`.
    if !processed.is_multiple_of(PROGRESS_EVERY) {
        on_progress(processed, total);
    }

    tx.commit()?;

    if !reasons.is_empty() {
        eprintln!(
            "[Import SCID] {} partie(s) ignorée(s) sur {} — détail par cause :",
            summary.skipped, total
        );
        let mut sorted: Vec<(&&str, &usize)> = reasons.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (label, count) in sorted {
            eprintln!("[Import SCID]   {count:>5} — {label}");
        }
    }

    Ok(summary)
}
