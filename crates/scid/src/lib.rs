//! Decoding of SCID databases (`.si4`/`.sn4`/`.sg4`, then `.si5`/
//! `.sn5`/`.sg5`) into PGN text, reusable afterward by
//! `db::reference_import::import_one` (see `crates/db`) with no
//! modification to the latter — this is the whole architectural principle
//! adopted (see `SUIVI_PLAN_ACTION.md`, "import SCID" discussion).
//!
//! This crate does NOT depend on `rusqlite` or Slint: it is a pure binary
//! format decoder, testable independently of the database and the
//! graphical interface. It depends only on `chess_core` (aliased
//! `chess_core`, see `Cargo.toml`) for the chess game
//! representation (position, legal moves, PGN text generation).
//!
//! # Scope (see `SUIVI_PLAN_ACTION.md`)
//!
//! - Both si4 AND si5 supported since 12/07/2026 (V2 Phase C1, task #21):
//!   `si4`/`si5` contain ONLY the readers specific to each disk format
//!   (index, name table) — everything else (moves decoder
//!   `moves`/`game_blob`, PGN assembly `pgn_build`, neutral structures
//!   `entry`/`names`) is entirely shared and UNCHANGED between the two,
//!   the content of a game (moves/tags/comments/variations/NAGs) being
//!   reused bit-for-bit identically between si4 and si5 (see
//!   `si5_specification_fr.txt` §"Historical context").
//! - Non-standard starting positions (custom FEN) supported since
//!   12/07/2026 (V2 Phase C2, task #22): see `game_blob`'s module doc
//!   (`build_piece_lists`) for the piece-index assignment algorithm,
//!   replayed from `Position::AddPiece` (`position.cpp`).
//! - Variations (nested to any depth) decoded since 13/07/2026
//!   (V2 Phase D, task #23): see `game_blob`'s module doc
//!   (`decode_variation_tree`) — the source SCID file's main line
//!   remains the main line (`children[0]`) of the imported tree, the
//!   variations are added normally under `children[1..]`, then exported
//!   as RAV `(...)` by `chess_core::pgn::export_pgn` (already tree-aware,
//!   no modification needed on the export side).
//!   `error::GameDecodeError::ContainsVariations` is no longer returned by
//!   this module (kept in the enum for compatibility/Display, same
//!   situation as `NonStandardStart` since Phase C2).
//!
//! The entire scope initially planned by the V2 evolution plan (Phases
//! A through D) is now implemented. A game can now only be skipped
//! for a REAL anomaly (corrupted stream, illegal move, out-of-bounds
//! offset...) — see [`error::GameDecodeError`], counted by the caller
//! (`crates/db`) exactly as the existing PGN import already does
//! (`ImportSummary`).

pub mod bytes;
pub mod dates;
pub mod eco;
pub mod entry;
pub mod error;
pub mod game_blob;
pub mod moves;
pub mod names;
pub mod pgn_build;
pub mod si4;
pub mod si5;

pub use error::{GameDecodeError, ScidError};
pub use si4::database::{Si4Database, Si4Paths};
pub use si5::database::{Si5Database, Si5Paths};
