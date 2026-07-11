//! Final assembly: `IndexEntry` + name tables + `.sg4`/`.sg5` blob of a
//! game -> full PGN text, ready for `db::reference_import::import_one`
//! (UNCHANGED — see the architecture discussion in `SUIVI_PLAN_ACTION.md`:
//! this module only produces PGN text, it never touches `SQLite`).
//!
//! `IndexEntry`/`NameTables` are NEUTRAL structures (`crate::entry`/
//! `crate::names`, extracted from `si4::index`/`si4::namebase` on
//! 12/07/2026, V2 Phase C1, task #21): this module therefore knows nothing
//! about the original disk format (si4 or si5) — it works identically for
//! both, as long as the caller provides it with these already-resolved
//! structures.

use std::fmt::Write as _;

use crate::dates;
use crate::eco;
use crate::entry::IndexEntry;
use crate::error::GameDecodeError;
use crate::game_blob;
use crate::names::NameTables;
use chess_core::pgn::{export_pgn, PgnTags};
use chess_core::types::game_state::GameResult;

/// Builds the full PGN text of a game described by `entry` (resolved
/// via `names`) whose moves are encoded in `blob` (the bytes
/// `[entry.offset, entry.offset + entry.length)` of the `.sg4`/`.sg5`, already
/// extracted by the caller — see `si4::database`).
///
/// # Errors
/// See [`GameDecodeError`]: the game is skipped (not the whole database) on
/// failure — variation encountered, non-standard position, illegal move...
pub fn build_pgn(entry: &IndexEntry, names: &NameTables, blob: &[u8]) -> Result<String, GameDecodeError> {
    let mut game = game_blob::decode_mainline(blob)?;

    // The result comes from the IndexEntry (§1.1: 0=none, 1=White, 2=Black,
    // 3=Draw), NOT from automatic checkmate/stalemate detection by `chess_core`
    // (most real games end by resignation, not by a terminal
    // state detectable on the board). `export_pgn` overwrites the Result
    // tag with `game.result` anyway (see `pgn.rs`), so it is THIS field
    // that needs to be set correctly, not the tags.
    game.result = match entry.result {
        1 => GameResult::WhiteWins,
        2 => GameResult::BlackWins,
        3 => GameResult::Draw,
        _ => GameResult::Ongoing,
    };

    let tags = PgnTags {
        event:  sanitize_tag_value(names.event(entry.event_id)),
        site:   sanitize_tag_value(names.site(entry.site_id)),
        date:   dates::date_to_pgn(entry.date),
        round:  sanitize_tag_value(names.round(entry.round_id)),
        white:  sanitize_tag_value(names.player(entry.white_id)),
        black:  sanitize_tag_value(names.player(entry.black_id)),
        result: String::new(), // overwritten by `export_pgn`, see above
    };

    let mut pgn = export_pgn(&game, Some(tags));
    splice_extra_tags(&mut pgn, entry);
    Ok(pgn)
}

/// Bugfix 12/07/2026: neutralizes characters that would break either the
/// shared PGN tokenizer (`chess_core::pgn::tokenize`, which skips a tag
/// `[...]` by advancing to the first `]` encountered, WITHOUT accounting for
/// quotes), or tag extraction by text search
/// (`db::import_export::extract_tag`, which stops at the first `"` encountered
/// to delimit the value). Diagnosed on a real `.si4` file: player/tournament
/// names containing a `]` caused the PGN revalidation of the entire game to
/// fail (11/500 in the test — see `SUIVI_PLAN_ACTION.md`).
///
/// Deliberately conservative choice (explicit user request: "the
/// safest solution... if we lose a few games in the import it's
/// not a big deal"): these characters are REMOVED from the value rather
/// than escaped. A real escaping system would require modifying
/// `chess_core::pgn` — code SHARED by the entire software (including
/// the application's own PGN export), hence a much larger blast
/// radius than this scid-only bugfix — for a marginal gain on
/// cases that are already rare.
/// Also removes `[` and line breaks out of symmetric caution, even
/// though neither was observed as a cause of rejection in the test.
fn sanitize_tag_value(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '"' | '[' | ']' | '\n' | '\r')).collect()
}

/// Inserts the ECO/WhiteElo/BlackElo tags — not handled by `export_pgn`
/// (Seven Tag Roster only, see `pgn.rs`) but extracted by
/// `db::reference_import::import_one` via a tag search directly
/// in the PGN text — right before the blank line that separates the tags from
/// the moves.
fn splice_extra_tags(pgn: &mut String, entry: &IndexEntry) {
    let mut extra = String::new();
    // `eco_to_string` comes from a fixed lookup table (`eco.rs`),
    // not from external data — sanitized only out of symmetric caution,
    // no real risk identified here.
    if let Some(eco_str) = eco::eco_to_string(entry.eco_code) {
        let _ = writeln!(extra, "[ECO \"{}\"]", sanitize_tag_value(&eco_str));
    }
    if entry.white_elo > 0 {
        let _ = writeln!(extra, "[WhiteElo \"{}\"]", entry.white_elo);
    }
    if entry.black_elo > 0 {
        let _ = writeln!(extra, "[BlackElo \"{}\"]", entry.black_elo);
    }
    if extra.is_empty() {
        return;
    }

    if let Some(pos) = pgn.find("\n\n") {
        pgn.insert_str(pos + 1, &extra);
    } else {
        // Should never happen: `export_pgn` always produces this
        // separator (see `pgn.rs`). Out of caution, we don't lose
        // the information rather than panicking or ignoring it.
        pgn.insert_str(0, &extra);
    }
}
