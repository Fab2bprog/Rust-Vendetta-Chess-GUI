//! Readers specific to the si5 disk format (`.si5`/`.sn5`) — see the `crate`
//! doc for the overview (V2 Phase C1, 12/07/2026, task #21).
//! The content of a game (`.sg5`) is decoded by `crate::game_blob`,
//! shared as-is with si4 — no `si5::game_blob` module here.

pub mod database;
pub mod index;
pub mod namebase;
