//! High-level orchestration for a si5 database: opening the three
//! files (`.si5`/`.sn5`/`.sg5`), and decoding a given game into PGN
//! text on demand — an exact mirror of `si4::database` (V2 Phase C1,
//! 12/07/2026, task #21), only the low-level readers (`index`/
//! `namebase` of this `si5` module) differ; `pgn_build`/the moves
//! decoder are shared as-is with si4.

use std::path::{Path, PathBuf};

use super::{index, namebase};
use crate::entry::IndexEntry;
use crate::error::{GameDecodeError, ScidError};
use crate::names::NameTables;
use crate::pgn_build;

/// The three file paths of a si5 database, derived from the `.si5` path.
#[derive(Debug, Clone)]
pub struct Si5Paths {
    pub index:    PathBuf,
    pub namebase: PathBuf,
    pub games:    PathBuf,
}

impl Si5Paths {
    /// Derives the three paths from the `.si5` file path (the other two
    /// files must share the same base name, alongside it).
    #[must_use]
    pub fn from_index_path(si5_path: &Path) -> Self {
        Self {
            index:    si5_path.to_path_buf(),
            namebase: si5_path.with_extension("sn5"),
            games:    si5_path.with_extension("sg5"),
        }
    }
}

/// A si5 database opened in memory: index fully decoded, names resolved,
/// and the games file loaded as-is (individual blobs are
/// extracted on demand via `game_pgn`).
pub struct Si5Database {
    entries: Vec<IndexEntry>,
    names:   NameTables,
    games:   Vec<u8>,
}

impl Si5Database {
    /// Opens and fully loads a si5 database into memory.
    ///
    /// # Errors
    /// See [`ScidError`]: truncated file, I/O error on one of the three
    /// files. Unlike si4, the si5 format has no verifiable magic byte
    /// (`.si5`/`.sn5` have no header, see `si5::index`/
    /// `si5::namebase`) — [`ScidError::BadMagic`] is therefore never returned
    /// here, a limitation of the format itself, not of this implementation.
    pub fn open(paths: &Si5Paths) -> Result<Self, ScidError> {
        let index_bytes = std::fs::read(&paths.index)?;
        let names_bytes = std::fs::read(&paths.namebase)?;
        let games_bytes = std::fs::read(&paths.games)?;

        let entries = index::read_all_entries(&index_bytes)?;
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
    /// `(offset, length)` exceeds the size of the `.sg5` file; the other
    /// variants of [`GameDecodeError`] come from decoding the blob
    /// itself (see `game_blob`/`pgn_build`, shared with si4).
    pub fn game_pgn(&self, n: usize) -> Result<String, GameDecodeError> {
        let entry = self.entries.get(n).ok_or(GameDecodeError::BadOffset)?;
        // V2 Phase C2 (12/07/2026, task #22): see the identical note in
        // `si4/database.rs` — the early filtering on `non_standard_start` was
        // removed, these games are now decoded normally.

        let start = usize::try_from(entry.offset).map_err(|_| GameDecodeError::BadOffset)?;
        let end = start
            .checked_add(entry.length as usize)
            .ok_or(GameDecodeError::BadOffset)?;
        let blob = self.games.get(start..end).ok_or(GameDecodeError::BadOffset)?;

        pgn_build::build_pgn(entry, &self.names, blob)
    }
}
