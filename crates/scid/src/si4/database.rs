//! High-level orchestration for a si4 database: opening the three
//! files (`.si4`/`.sn4`/`.sg4`), and decoding a given game into PGN
//! text on demand (see `crates/db` for the full import loop —
//! task #6, this module knows nothing about `SQLite`).

use std::path::{Path, PathBuf};

use super::{index, namebase};
use crate::entry::IndexEntry;
use crate::error::{GameDecodeError, ScidError};
use crate::names::NameTables;
use crate::pgn_build;

/// The three file paths of a si4 database, derived from the `.si4` path.
#[derive(Debug, Clone)]
pub struct Si4Paths {
    pub index:    PathBuf,
    pub namebase: PathBuf,
    pub games:    PathBuf,
}

impl Si4Paths {
    /// Derives the three paths from the `.si4` file path (the other two
    /// files must share the same base name, alongside it).
    #[must_use]
    pub fn from_index_path(si4_path: &Path) -> Self {
        Self {
            index:    si4_path.to_path_buf(),
            namebase: si4_path.with_extension("sn4"),
            games:    si4_path.with_extension("sg4"),
        }
    }
}

/// A si4 database opened in memory: index fully decoded, names resolved,
/// and the games file loaded as-is (individual blobs are
/// extracted on demand via `game_pgn`).
pub struct Si4Database {
    entries: Vec<IndexEntry>,
    names:   NameTables,
    games:   Vec<u8>,
}

impl Si4Database {
    /// Opens and fully loads a si4 database into memory.
    ///
    /// # Errors
    /// See [`ScidError`]: invalid magic/version, truncated file, I/O
    /// error on one of the three files.
    pub fn open(paths: &Si4Paths) -> Result<Self, ScidError> {
        let index_bytes = std::fs::read(&paths.index)?;
        let names_bytes = std::fs::read(&paths.namebase)?;
        let games_bytes = std::fs::read(&paths.games)?;

        let header  = index::read_header(&index_bytes)?;
        let entries = index::read_all_entries(&index_bytes, &header)?;
        let names   = namebase::read_namebase(&names_bytes)?;

        Ok(Self { entries, names, games: games_bytes })
    }

    /// Number of games in the database.
    #[must_use]
    pub fn game_count(&self) -> usize {
        self.entries.len()
    }

    /// Decodes game number `n` (0-based) into full PGN text.
    ///
    /// # Errors
    /// [`GameDecodeError::BadOffset`] if `n` is out of bounds or if
    /// `(offset, length)` exceeds the size of the `.sg4` file; the other
    /// variants of [`GameDecodeError`] come from decoding the blob itself
    /// (see `game_blob`/`pgn_build`).
    pub fn game_pgn(&self, n: usize) -> Result<String, GameDecodeError> {
        let entry = self.entries.get(n).ok_or(GameDecodeError::BadOffset)?;
        // V2 Phase C2 (12/07/2026, task #22): non-standard starting
        // positions are now decoded by `game_blob::decode_mainline`
        // (see `build_piece_lists`) — the early filtering on
        // `entry.non_standard_start` was removed, as it wrongly rejected
        // games that are now supported.

        let start = usize::try_from(entry.offset).map_err(|_| GameDecodeError::BadOffset)?;
        let end = start
            .checked_add(entry.length as usize)
            .ok_or(GameDecodeError::BadOffset)?;
        let blob = self.games.get(start..end).ok_or(GameDecodeError::BadOffset)?;

        pgn_build::build_pgn(entry, &self.names, blob)
    }
}
