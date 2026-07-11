//! Resolved name tables (players, tournaments, sites, rounds) — a NEUTRAL
//! structure, independent of the original disk format (`.sn4` front-coded/
//! sorted, vs `.sn5` append-only journal/varint — see `si4::namebase`/
//! `si5::namebase` for the format-specific readers).
//!
//! Extracted from `si4::namebase` (12/07/2026, V2 Phase C1, task #21) for the
//! same reason as `entry::IndexEntry`: `pgn_build::build_pgn` only knows
//! this structure, never the original disk format.

/// The 4 reconstructed name tables, indexed by `idNumberT` (the ID stored
/// in the `IndexEntry` records of the index file).
#[derive(Debug, Clone, Default)]
pub struct NameTables {
    pub players: Vec<String>,
    pub events:  Vec<String>,
    pub sites:   Vec<String>,
    pub rounds:  Vec<String>,
}

impl NameTables {
    #[must_use]
    pub fn player(&self, id: u32) -> &str {
        self.players.get(id as usize).map_or("?", String::as_str)
    }
    #[must_use]
    pub fn event(&self, id: u32) -> &str {
        self.events.get(id as usize).map_or("?", String::as_str)
    }
    #[must_use]
    pub fn site(&self, id: u32) -> &str {
        self.sites.get(id as usize).map_or("?", String::as_str)
    }
    #[must_use]
    pub fn round(&self, id: u32) -> &str {
        self.rounds.get(id as usize).map_or("?", String::as_str)
    }
}
