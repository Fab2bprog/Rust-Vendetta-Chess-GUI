//! Game controller for the GUI.
//!
//! Bridges the chess logic (`chess_core`) with the Slint data models
//! ([`crate::SquareData`], [`crate::MoveRow`]).
//!
//! # Lifecycle
//!
//! ```text
//! GameController::new()
//!   └─ build_squares()       → Vec<SquareData>   (initial display)
//!   └─ build_move_rows()     → Vec<MoveRow>      (empty list at start)
//!
//! on_click(row, col) → bool  (true = re-render needed)
//!   ├─ if in history-viewing mode:
//!   │   ├─ variation_editing == false → return to the current position
//!   │   └─ variation_editing == true  → handled against the viewed position
//!   │       (selection then target square = new variation, see Step 5)
//!   ├─ selecting a piece → targets = legal moves from that square
//!   ├─ click on a target   → play() + last_move updated
//!   │   └─ if promotion: pending_promotion = Some((from, to)), not yet played
//!   └─ irrelevant click   → deselection
//!
//! complete_promotion(piece: i32) → bool
//!   └─ plays the promotion move with the chosen piece (1=Queen…4=Knight)
//!
//! go_to_ply(ply) → bool      (true if the state changed)
//!   └─ viewed_ply = Some(ply), rebuild squares from position_at(ply+1)
//!
//! set_variation_mode_enabled(bool)  (PHASE 16, Step 5)
//!   └─ directly sets the same field as enter/exit_variation_editing
//!      below, without the `viewed_ply.is_some()` precondition. Since
//!      PHASE 26, Step 3, `main.rs` no longer calls this method (wired to
//!      the "Create/End variation" buttons instead) — kept
//!      as a low-level setter for this module's unit tests.
//!
//! enter_variation_editing() → bool / exit_variation_editing() / is_variation_editing() → bool
//!   (PHASE 26, Step 1) — an explicit, self-contained state, triggered by a
//!   user gesture ("Create a variation" / "End the variation")
//!   rather than derived implicitly from viewing history or the end of the game.
//!   Stays active until `exit_variation_editing()` is called,
//!   including after several moves are played within the variation (unlike
//!   `viewed_ply`, which goes back to `None` as soon as the first move is played).
//!
//! reset()                    → returns to the initial position
//! build_squares()            → Vec<SquareData>   (after each change)
//! build_move_rows()          → Vec<MoveRow>      (after each move played)
//! viewed_ply_slint()         → i32               (-1 or the index of the viewed ply)
//! status_key()               → i18n key for the status bar
//! end_reason_key()           → i18n key for the game-over reason
//! is_white_turn()            → bool
//! has_pending_promotion()    → bool
//! pending_promo_is_white()   → bool
//! ```

// The i32→u8 conversions after bounds checking are intentional.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]

use std::collections::HashSet;

use chess_core::{
    game::GameState as ChessGame,
    movegen::{generate_legal_moves, is_in_check, is_square_attacked},
    notation::move_to_san,
    rules::{game_status, make_move, GameStatus},
    types::{
        chess_move::{Move, MoveKind},
        game_state::GameResult,
        piece::{Color, Piece, PieceKind},
        position::Position,
        square::Square,
    },
};

use crate::{CapturedPieceData, MoveRow, SquareData};

// ── Coordinate conversions ──────────────────────────────────────────────────

/// Converts Slint `(row, col)` coordinates to a [`Square`].
///
/// | Slint | Chess core |
/// |-------|-----------|
/// | row 0 | rank 8 (rank index 7) |
/// | row 7 | rank 1 (rank index 0) |
/// | col 0 | file a (file 0)    |
/// | col 7 | file h (file 7)    |
fn slint_to_square(row: i32, col: i32) -> Square {
    Square::new(col as u8, (7 - row) as u8)
}

/// Sorted algebraic notation of a set of squares — for a deterministic JSON
/// log (debug mode), since `HashSet<Square>` has no stable order.
fn sorted_algebraic(squares: &HashSet<Square>) -> Vec<String> {
    let mut v: Vec<String> = squares.iter().map(|s| s.to_algebraic()).collect();
    v.sort();
    v
}

// ── SVG piece identifiers ────────────────────────────────────────────────────
//
// Each piece is identified by a 2-character ASCII code:
//   first character  : 'w' (white) or 'b' (black)
//   second character  : 'K' 'Q' 'R' 'B' 'N' 'P'
// These identifiers match the SVG file names in assets/pieces/.

fn piece_id(piece: Piece) -> &'static str {
    match (piece.color, piece.kind) {
        (Color::White, PieceKind::King)   => "wK",
        (Color::White, PieceKind::Queen)  => "wQ",
        (Color::White, PieceKind::Rook)   => "wR",
        (Color::White, PieceKind::Bishop) => "wB",
        (Color::White, PieceKind::Knight) => "wN",
        (Color::White, PieceKind::Pawn)   => "wP",
        (Color::Black, PieceKind::King)   => "bK",
        (Color::Black, PieceKind::Queen)  => "bQ",
        (Color::Black, PieceKind::Rook)   => "bR",
        (Color::Black, PieceKind::Bishop) => "bB",
        (Color::Black, PieceKind::Knight) => "bN",
        (Color::Black, PieceKind::Pawn)   => "bP",
    }
}

/// Builds a STATIC board (no selection, no legal moves, no
/// assist badge) from an arbitrary position — for the non-interactive
/// board preview in `GameDetailView` (ergonomics bugfix
/// 09/07/2026, "Détail de la partie"). Unlike
/// [`GameController::build_squares`] (a method coupled to the state of an
/// interactive game in progress — selection, legal moves, Assist mode), this
/// free function only depends on a [`Position`] and the last move played
/// to reach it — reusable for any historical position
/// (e.g. a game from the reference database), not only the current
/// game. King in check / checkmate is still flagged (immediate computation,
/// visual consistency with the main board).
#[must_use]
pub fn build_static_squares(pos: &Position, last_move: Option<(Square, Square)>) -> Vec<SquareData> {
    let check_square: Option<Square> =
        if is_in_check(pos) { pos.board.find_king(pos.side_to_move) } else { None };
    let mated_square: Option<Square> =
        if check_square.is_some() && game_status(pos) == GameStatus::Checkmate {
            check_square
        } else {
            None
        };
    let (last_from, last_to) = match last_move {
        Some((from, to)) => (Some(from), Some(to)),
        None => (None, None),
    };

    let mut squares = Vec::with_capacity(64);
    for row in 0..8_i32 {
        for col in 0..8_i32 {
            let sq    = slint_to_square(row, col);
            let piece = pos.board.piece_at(sq);

            let (piece_char, piece_side) = match piece {
                Some(p) => (
                    slint::SharedString::from(piece_id(p)),
                    if p.color == Color::White { 1_i32 } else { 2_i32 },
                ),
                None => (slint::SharedString::from(""), 0_i32),
            };

            squares.push(SquareData {
                row,
                col,
                piece_char,
                is_light:         (row + col) % 2 == 0,
                piece_side,
                is_selected:      false,
                is_legal_target:  false,
                is_last_from:     last_from == Some(sq),
                is_last_to:       last_to   == Some(sq),
                is_king_in_check: check_square == Some(sq),
                is_capture_risk:  false,
                is_gives_check:   false,
                is_gives_mate:    false,
                is_mated_king:    mated_square == Some(sq),
            });
        }
    }
    squares
}

/// Summary of captures (trophies + differential) up to half-move `ply`
/// (exclusive, 0-based: `ply` moves already played), for an ARBITRARY game
/// (ergonomics follow-up 10/07/2026: "Enregistrer l'échiquier en image
/// (PNG)" button of "Détail de la partie"). Exactly the same logic as
/// [`GameController::captured_summary`] (identical capture detection,
/// same sort, same differential computation), but free of any interactive
/// controller state (`viewed_ply`) — applicable to any
/// reference game loaded via `chess_core::pgn::import_pgn`, at the ply
/// requested by the caller rather than the "current" position of a
/// `GameController`.
#[must_use]
pub fn captured_summary_at_ply(
    game: &ChessGame,
    ply: usize,
) -> (Vec<CapturedPieceData>, Vec<CapturedPieceData>, i32) {
    let mut captured_white = Vec::new();
    let mut captured_black = Vec::new();

    for record in game.history().records().iter().take(ply) {
        let Ok(pos_before) = Position::from_fen(&record.fen_before) else { continue };
        let captured = match record.mv.kind {
            MoveKind::EnPassant => {
                let sq = Square::new(record.mv.to.file(), record.mv.from.rank());
                pos_before.board.piece_at(sq)
            }
            _ => pos_before.board.piece_at(record.mv.to),
        };
        if let Some(p) = captured {
            match p.color {
                Color::White => captured_white.push(p.kind),
                Color::Black => captured_black.push(p.kind),
            }
        }
    }

    captured_white.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));
    captured_black.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));

    let white_material_lost: i32 = captured_white.iter().copied().map(piece_value).sum();
    let black_material_lost: i32 = captured_black.iter().copied().map(piece_value).sum();

    let white_trophies = GameController::compact_captures(&captured_black, Color::Black);
    let black_trophies = GameController::compact_captures(&captured_white, Color::White);
    let diff = black_material_lost - white_material_lost;

    (white_trophies, black_trophies, diff)
}

/// Summary of captures (trophies + differential) along a PATH of UCI moves
/// from the standard starting position — ergonomics follow-up 10/07/2026,
/// the "Filtrer par ouverture" tab's preview board. Exactly the same
/// logic as [`captured_summary_at_ply`] (identical capture detection —
/// including en passant —, same sort, same differential
/// computation), but replays `path` itself instead of reading
/// `ChessGame::history()`: the opening tree has no underlying [`ChessGame`],
/// only a list of UCI moves (see `main.rs`,
/// `replay_opening_tree_path`). Some redundant computation with the
/// latter (which also replays `path` for the position/breadcrumb) is
/// accepted here rather than coupling the two functions across modules —
/// the same choice already made for [`captured_summary_at_ply`] relative to
/// `GameController::captured_summary` (see its documentation).
///
/// Silently stops at the first invalid move of the path (should
/// never happen, each move comes from an already-validated
/// [`crate::OpeningMoveRow::uci`]) rather than panicking — the same defensive
/// stance as `replay_opening_tree_path`.
#[must_use]
pub fn captured_summary_for_path(path: &[String]) -> (Vec<CapturedPieceData>, Vec<CapturedPieceData>, i32) {
    let mut captured_white = Vec::new();
    let mut captured_black = Vec::new();
    let mut pos = Position::starting();

    for uci in path {
        let Some(raw) = Move::from_uci(uci) else { break };
        let Some(mv) = generate_legal_moves(&pos).into_iter().find(|m| {
            m.from == raw.from && m.to == raw.to && m.promotion == raw.promotion
        }) else {
            break;
        };

        let captured = match mv.kind {
            MoveKind::EnPassant => {
                let sq = Square::new(mv.to.file(), mv.from.rank());
                pos.board.piece_at(sq)
            }
            _ => pos.board.piece_at(mv.to),
        };
        if let Some(p) = captured {
            match p.color {
                Color::White => captured_white.push(p.kind),
                Color::Black => captured_black.push(p.kind),
            }
        }

        let Ok(next_pos) = make_move(&pos, mv) else { break };
        pos = next_pos;
    }

    captured_white.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));
    captured_black.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));

    let white_material_lost: i32 = captured_white.iter().copied().map(piece_value).sum();
    let black_material_lost: i32 = captured_black.iter().copied().map(piece_value).sum();

    let white_trophies = GameController::compact_captures(&captured_black, Color::Black);
    let black_trophies = GameController::compact_captures(&captured_white, Color::White);
    let diff = black_material_lost - white_material_lost;

    (white_trophies, black_trophies, diff)
}

/// Converts the best move suggested by a UCI engine (raw UCI notation,
/// e.g. `"g1f3"`, `"e7e8q"`) into SAN notation for position `pos` (ergonomics
/// follow-up 10/07/2026: "Détail de la partie" info block).
///
/// `Move::from_uci` alone is not enough: it never fills in
/// `MoveKind::Castle` or `MoveKind::EnPassant` (always `Normal` or
/// `Promotion`), which would produce an incorrect SAN for castling
/// (`notation::move_to_san` explicitly tests `m.kind == MoveKind::Castle`
/// to write "O-O"/"O-O-O") or for an en-passant capture (empty destination
/// square, so not detected as a capture without `MoveKind::EnPassant`). So
/// the matching legal move (same origin/destination/promotion) is looked up
/// in `generate_legal_moves(pos)`, which carries the correct `MoveKind`, before
/// converting it. Returns `None` if the UCI string is invalid or does not
/// correspond to any legal move of `pos` (inconsistent position/move —
/// should not happen in practice, but no panic in that case).
#[must_use]
pub fn best_move_san(pos: &Position, uci: &str) -> Option<String> {
    let parsed = Move::from_uci(uci)?;
    let resolved = generate_legal_moves(pos).into_iter().find(|m| {
        m.from == parsed.from && m.to == parsed.to && m.promotion == parsed.promotion
    })?;
    Some(move_to_san(pos, resolved))
}

// ── Captured pieces ────────────────────────────────────────────────────────────
//
// Computed on demand from the history (fen_before of each move), with no
// incremental cache: the volume (a few dozen moves per game) makes this
// recomputation negligible in practice on every display refresh.

/// Conventional value of a piece for sorting and the material differential
/// shown in the capture strips (standard scale: P=1,
/// N/B=3, R=5, Q=9).
fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::Pawn                       => 1,
        PieceKind::Knight | PieceKind::Bishop => 3,
        PieceKind::Rook                       => 5,
        PieceKind::Queen                      => 9,
        PieceKind::King                       => 0, // should never be captured
    }
}

// ── Controller ───────────────────────────────────────────────────────────────

/// Controller for the current game.
///
/// Wraps [`chess_core::game::GameState`] and manages piece selection,
/// computing the displayable legal moves, executing moves via clicks,
/// deferred promotion, history navigation, and end of game.
pub struct GameController {
    game:       ChessGame,
    /// Currently selected square (None = nothing selected).
    selection:  Option<Square>,
    /// Target squares of the legal moves from `selection`.
    targets:    HashSet<Square>,
    /// Last move played (for the green overlay display in the current position).
    last_move:  Option<Move>,
    /// Half-move viewed in the history.
    /// `None`   = current position (end of game).
    /// `Some(p)` = view the position after half-move index `p` (0-based).
    viewed_ply: Option<usize>,
    /// Pending promotion: (from, to) of the pawn move that reached the 8th rank.
    /// `None` if no promotion is pending.
    pending_promotion: Option<(Square, Square)>,
    /// Color of the pawn that is promoting (to display the correct pieces).
    pending_promo_is_white: bool,
    /// Original ply of the pending promotion when it was initiated in
    /// history-viewing mode (PHASE 16, Step 5): `Some(ply)` means that
    /// [`Self::complete_promotion`] must branch a variation via
    /// [`chess_core::game::GameState::play_variation`] rather than play at
    /// the tip of the game. `None` = normal promotion at the current position.
    pending_promotion_ply: Option<usize>,
    /// Allows or disallows creating variations by playing a move from a
    /// historical position (PHASE 16, Step 5, decision 3; became an
    /// explicit, self-contained state in PHASE 26, Step 1). `true` as long as
    /// [`Self::enter_variation_editing`] has been called and
    /// [`Self::exit_variation_editing`] has not yet been — stays true
    /// after several moves are played within the variation, unlike
    /// `viewed_ply` which goes back to `None` as soon as the first move is played
    /// (see [`Self::execute_variation_move`]). Can also be driven directly
    /// by [`Self::set_variation_mode_enabled`] (a low-level setter kept
    /// for tests, no longer used by `main.rs` since PHASE 26, Step 3).
    /// `false` by default.
    variation_editing: bool,
    /// PHASE 68: `true` = Assist mode active (💡 button of the
    /// icon bar). When active AND a piece is selected,
    /// [`Self::build_squares`] computes three extra indicators
    /// for each target square (capture risk, check, checkmate)
    /// — see the `is_capture_risk`/`is_gives_check`/`is_gives_mate` fields
    /// of [`SquareData`]. `false` by default, and always forced to `false`
    /// on the Rust side (`main.rs`) in Puzzle mode.
    assist_mode: bool,
}

impl GameController {
    /// Creates a new controller at the starting position.
    #[must_use]
    pub fn new() -> Self {
        Self {
            game:                   ChessGame::new(),
            selection:              None,
            targets:                HashSet::new(),
            last_move:              None,
            viewed_ply:             None,
            pending_promotion:      None,
            pending_promo_is_white: false,
            pending_promotion_ply:  None,
            variation_editing:      false,
            assist_mode:            false,
        }
    }

    // ── Click handling ────────────────────────────────────────────────────────

    /// Handles a click on the Slint square `(row, col)`.
    ///
    /// Returns `true` if the state changed and a re-render is needed.
    ///
    /// If a promotion move is detected, `has_pending_promotion()` returns
    /// `true` after this call; the promotion is not played until
    /// `complete_promotion()` is called.
    pub fn on_click(&mut self, row: i32, col: i32) -> bool {
        crate::debug_log::log_event("on_click", &serde_json::json!({
            "row": row,
            "col": col,
            "viewed_ply": self.viewed_ply,
            "variation_editing": self.variation_editing,
            "selection": self.selection.map(Square::to_algebraic),
            "targets": sorted_algebraic(&self.targets),
        }));
        // In history mode: see `on_click_history` (PHASE 16, Step 5) —
        // either an immediate return to the current position (historical
        // behavior, if variation creation is not available in the
        // current context), or a click handled against the
        // viewed position (selection/move → variation).
        if let Some(ply) = self.viewed_ply {
            crate::debug_log::log_event("on_click_routed_to_history", &serde_json::json!({ "ply": ply }));
            return self.on_click_history(ply, row, col);
        }

        // Game over → clicks do nothing
        if self.game.is_over() {
            return false;
        }

        let sq   = slint_to_square(row, col);
        let side = self.game.position().side_to_move;

        // ── Case 1: a piece is already selected ──────────────────────────────
        if let Some(from) = self.selection {
            // The clicked square is a legal target → play the move
            if self.targets.contains(&sq) {
                if self.execute_move(from, sq) {
                    return true;
                }
                // execute_move returned false → promotion pending
                if self.pending_promotion.is_some() {
                    return true; // re-render to show the modal
                }
            }

            // Click on the same square again → deselect
            if sq == from {
                self.clear_selection();
                return true;
            }

            // Click on another piece of the side to move → change selection
            if let Some(p) = self.game.position().board.piece_at(sq) {
                if p.color == side {
                    self.select(sq);
                    return true;
                }
            }

            // Irrelevant click → deselect
            self.clear_selection();
            return true;
        }

        // ── Case 2: nothing selected — select a piece of the side to move ────
        if let Some(p) = self.game.position().board.piece_at(sq) {
            if p.color == side {
                self.select(sq);
                return true;
            }
        }

        // Empty square or opposing piece with no active selection → nothing to do
        false
    }

    // ── Click in history mode (PHASE 16, Step 5) ──────────────────────────────

    /// Allows or disallows creating variations by playing a move from a
    /// historical position (PHASE 16 decision #3).
    ///
    /// Low-level setter: directly sets the same field as
    /// [`Self::enter_variation_editing`]/[`Self::exit_variation_editing`],
    /// without their precondition/side effect (`viewed_ply.is_some()` on
    /// entry, resetting `viewed_ply` to `None` on exit). `main.rs`
    /// now exclusively drives `enter`/`exit_variation_editing` from
    /// the "Create/End variation" banner (PHASE 26, Step 3); this
    /// method remains useful for this module's unit tests, which don't
    /// need those side effects. As long as this mode is not enabled,
    /// [`Self::on_click`] keeps its historical behavior in viewing
    /// mode: any click returns to the current position.
    pub fn set_variation_mode_enabled(&mut self, enabled: bool) {
        self.variation_editing = enabled;
    }

    /// Enables or disables Assist mode (PHASE 68, 💡 button of the
    /// icon bar). Has no visible effect until the next
    /// [`Self::build_squares`] — `main.rs` rebuilds `squares` right after
    /// calling this setter, for an immediate refresh of the badges
    /// if a piece is already selected.
    pub fn set_assist_mode(&mut self, active: bool) {
        self.assist_mode = active;
    }

    /// Enters variation-editing mode (PHASE 26, Step 1).
    ///
    /// Unlike the previous implicit driving
    /// ([`Self::set_variation_mode_enabled`], derived from viewing history or
    /// the end of the game on the `main.rs` side), this mode is now
    /// triggered explicitly by a user gesture ("Create a
    /// variation" button), and stays active until [`Self::exit_variation_editing`]
    /// is called — including after several moves are played within the
    /// variation, since only `viewed_ply` (not this mode) is reset
    /// by [`Self::execute_variation_move`] on each move.
    ///
    /// Does nothing and returns `false` if no history move is
    /// currently being viewed (`viewed_ply` is `None`): there is then nothing
    /// to create a variation from.
    pub fn enter_variation_editing(&mut self) -> bool {
        crate::debug_log::log_event("enter_variation_editing_called", &serde_json::json!({
            "viewed_ply": self.viewed_ply,
            "selection_residuelle_avant": self.selection.map(Square::to_algebraic),
            "targets_residuels_avant": sorted_algebraic(&self.targets),
        }));
        if self.viewed_ply.is_none() {
            crate::debug_log::log_event("enter_variation_editing_refused", &serde_json::json!({ "raison": "viewed_ply_none" }));
            return false;
        }
        self.variation_editing = true;
        // Bugfix (user feedback 04/07/2026): any residual selection
        // (piece + target squares) predating entry into this
        // mode must be cleared here, symmetrically to
        // `Self::exit_variation_editing`. Without this, the very first click on
        // the board after clicking "Create a variation" could be
        // interpreted as a click on a target square for a piece already
        // selected before entering editing mode, causing a move
        // to be executed "on its own" without the user's knowledge.
        self.clear_selection();
        crate::debug_log::log_event("enter_variation_editing_accepted", &serde_json::json!({}));
        true
    }

    /// Ends variation-editing mode and returns to the current
    /// position, i.e. the tip of the active line ("End the variation"
    /// button).
    ///
    /// Always safe to call, even if the mode was not active or if
    /// history was not being viewed.
    pub fn exit_variation_editing(&mut self) {
        crate::debug_log::log_event("exit_variation_editing_called", &serde_json::json!({}));
        self.variation_editing = false;
        self.viewed_ply         = None;
        self.clear_selection();
    }

    /// `true` if variation-editing mode (PHASE 26, Step 1) is
    /// currently active.
    #[must_use]
    pub fn is_variation_editing(&self) -> bool {
        self.variation_editing
    }

    /// Handles a click while half-move `ply` of the history is being
    /// viewed (`self.viewed_ply == Some(ply)`).
    ///
    /// If variation creation is not available
    /// (`!self.variation_editing`), any click simply returns to the
    /// current position — identical behavior to before Step 5.
    ///
    /// Otherwise, the click is handled like a normal play click (selection then
    /// target square) but against the **viewed** position rather than the
    /// current position: playing a move creates a variation from this point in
    /// the history (decision 1 — never truncating the
    /// existing line) without leaving history mode until the move is
    /// played. A click that doesn't match any valid selection/move returns
    /// to the current position, as before Step 5.
    fn on_click_history(&mut self, ply: usize, row: i32, col: i32) -> bool {
        crate::debug_log::log_event("on_click_history", &serde_json::json!({
            "ply": ply,
            "row": row,
            "col": col,
            "variation_editing": self.variation_editing,
            "selection": self.selection.map(Square::to_algebraic),
            "targets": sorted_algebraic(&self.targets),
        }));

        if !self.variation_editing {
            crate::debug_log::log_event("on_click_history_editing_disabled_return_to_current", &serde_json::json!({}));
            self.viewed_ply = None;
            self.clear_selection();
            return true;
        }

        let pos = self.game
            .position_at(ply + 1)
            .unwrap_or_else(|| self.game.position().clone());
        let sq   = slint_to_square(row, col);
        let side = pos.side_to_move;
        crate::debug_log::log_event("on_click_history_square_clicked", &serde_json::json!({
            "square": sq.to_algebraic(),
            "side_to_move": format!("{side:?}"),
        }));

        if let Some(from) = self.selection {
            if self.targets.contains(&sq) {
                crate::debug_log::log_event("on_click_history_legal_target", &serde_json::json!({
                    "from": from.to_algebraic(),
                    "to": sq.to_algebraic(),
                    "ply": ply,
                }));
                if self.execute_variation_move(ply, from, sq) {
                    return true;
                }
                // execute_variation_move returned false → promotion pending
                if self.pending_promotion.is_some() {
                    crate::debug_log::log_event("on_click_history_promotion_pending", &serde_json::json!({}));
                    return true;
                }
            }

            if sq == from {
                crate::debug_log::log_event("on_click_history_deselect_same_square", &serde_json::json!({ "square": sq.to_algebraic() }));
                self.clear_selection();
                return true;
            }

            if let Some(p) = pos.board.piece_at(sq) {
                if p.color == side {
                    crate::debug_log::log_event("on_click_history_reselect", &serde_json::json!({
                        "square": sq.to_algebraic(),
                        "piece": format!("{p:?}"),
                        "previous_selection": from.to_algebraic(),
                    }));
                    self.select_from(&pos, sq);
                    return true;
                }
            }

            // Bugfix (user feedback 04/07/2026, real bug confirmed by the
            // diagnostic log): as long as variation editing is
            // active, an irrelevant click (wrong color, square with no
            // legal move) must NEVER silently return to the current
            // position — only explicit exit (the "End the
            // variation" button, `exit_variation_editing`) should end the
            // viewing. The old behavior ("as before Step 5",
            // inherited from before the explicit editing mode) reset
            // `viewed_ply` to `None` while leaving `variation_editing` active
            // : the banner remained displayed as if a variation were still
            // being edited, but any following click was actually handled
            // against the **live current** position — a move could
            // then actually be played in the game without the user
            // intending it (reported symptom: "a piece moves by
            // itself").
            crate::debug_log::log_event("on_click_history_irrelevant_click_with_selection", &serde_json::json!({ "square": sq.to_algebraic() }));
            self.clear_selection();
            return true;
        }

        if let Some(p) = pos.board.piece_at(sq) {
            if p.color == side {
                crate::debug_log::log_event("on_click_history_select_no_prior_selection", &serde_json::json!({
                    "square": sq.to_algebraic(),
                    "piece": format!("{p:?}"),
                }));
                self.select_from(&pos, sq);
                return true;
            }
        }

        // See the bugfix above: empty square/opposing piece with no
        // selection, while editing a variation → do nothing other
        // than a deselection (already empty here), `viewed_ply` stays unchanged.
        crate::debug_log::log_event("on_click_history_irrelevant_click_no_selection", &serde_json::json!({ "square": sq.to_algebraic() }));
        false
    }

    // ── Deferred promotion ────────────────────────────────────────────────────

    /// `true` if a promotion move is waiting for the user to choose the piece.
    #[must_use]
    pub fn has_pending_promotion(&self) -> bool {
        self.pending_promotion.is_some()
    }

    /// Color of the pawn that is promoting (`true` = white, `false` = black).
    ///
    /// Undefined value if `has_pending_promotion()` is false.
    #[must_use]
    pub fn pending_promo_is_white(&self) -> bool {
        self.pending_promo_is_white
    }

    /// Plays the pending promotion move with the chosen piece.
    ///
    /// `piece_code`: 1 = Queen, 2 = Rook, 3 = Bishop, 4 = Knight.
    ///
    /// PHASE 16, Step 5: if the promotion was initiated in history mode
    /// (with variation creation available, see [`Self::execute_variation_move`]),
    /// the promotion move is played as a variation from this ply rather than at
    /// the tip of the game, and the controller leaves history mode.
    ///
    /// Returns `true` if the move was played successfully.
    pub fn complete_promotion(&mut self, piece_code: i32) -> bool {
        let Some((from, to)) = self.pending_promotion.take() else {
            return false;
        };
        let variation_ply = self.pending_promotion_ply.take();

        let target_kind = match piece_code {
            2 => PieceKind::Rook,
            3 => PieceKind::Bishop,
            4 => PieceKind::Knight,
            _ => PieceKind::Queen, // 1 or any other code → Queen (most common choice)
        };

        let pos = match variation_ply {
            Some(ply) => self.game.position_at(ply + 1).unwrap_or_else(|| self.game.position().clone()),
            None      => self.game.position().clone(),
        };

        let legal = generate_legal_moves(&pos);
        let mv = legal.iter()
            .find(|m| m.from == from && m.to == to && m.promotion == Some(target_kind))
            .copied();

        let Some(mv) = mv else { return false; };

        let played = if let Some(ply) = variation_ply {
            self.game.play_variation(ply, mv).is_ok()
        } else {
            // Code audit 04/07/2026, point 4: `mv` always comes from
            // `generate_legal_moves`, so `play()` should never
            // fail — a `debug_assert!` documents this assumption and
            // surfaces the anomaly in debug builds (harmless in release, where
            // the result is still ignored as before) rather than a silent
            // desync between the display and the actual position.
            let result = self.game.play(mv);
            debug_assert!(result.is_ok(), "coup légal qui échoue à jouer : {mv:?}");
            true
        };
        if !played {
            return false;
        }

        self.last_move = Some(mv);
        crate::debug_log::log_event("move_played", &serde_json::json!({
            "source": if variation_ply.is_some() { "variation_promotion" } else { "human_live_promotion" },
            "from": from.to_algebraic(),
            "to": to.to_algebraic(),
            "san": self.last_move_san(),
            "uci": mv.to_uci(),
            "move_count": self.game.move_count(),
        }));
        if variation_ply.is_some() {
            self.viewed_ply = None;
        }
        self.clear_selection();
        true
    }

    // ── History navigation ────────────────────────────────────────────────────

    /// Requests to view the position after half-move index `ply`.
    ///
    /// - `ply < 0` or `ply >= move_count` → return to the current position.
    /// - Returns `true` if the state changed.
    pub fn go_to_ply(&mut self, ply: i32) -> bool {
        let move_count = self.game.move_count();

        if ply < 0 || (ply as usize) >= move_count {
            // Return to the current position
            if self.viewed_ply.is_some() {
                self.viewed_ply = None;
                self.clear_selection();
                return true;
            }
            return false;
        }

        let new_view = Some(ply as usize);
        if self.viewed_ply != new_view {
            self.viewed_ply = new_view;
            self.clear_selection();
            return true;
        }
        false
    }

    /// Index of the viewed half-move in Slint representation (`i32`).
    ///
    /// Returns `-1` if at the current position.
    // Clippy: `#[allow(cast_possible_wrap)]` — a half-move index can
    // never in practice approach `i32::MAX` (billions of moves).
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn viewed_ply_slint(&self) -> i32 {
        self.viewed_ply.map_or(-1_i32, |p| p as i32)
    }

    // ── Variation tree (PHASE 16, Step 3) ─────────────────────────────────────
    //
    // Additive API: changes nothing about current behavior (`viewed_ply`
    // remains the source of truth for `build_squares`/`build_move_rows`).
    // Prepares the notion of "current node" and flattening the tree with
    // depth/parentage for displaying variations (Step 4) and their
    // interactive creation (Step 5, see `on_click_history`).

    /// Node identifier (in the underlying move tree) corresponding
    /// to the currently displayed position: the node at `viewed_ply` if a ply
    /// is being viewed, otherwise the tip of the game (last move played).
    ///
    /// `None` if no move has been played yet.
    #[must_use]
    pub fn current_node_id(&self) -> Option<usize> {
        match self.viewed_ply {
            Some(ply) => self.game.history().node_id_at(ply),
            None => self.game.history().last_node_id(),
        }
    }

    /// Flattens the move tree into a list of [`chess_core::game_tree::FlatNode`]
    /// with depth and parentage — building block for displaying
    /// variations (Step 4). Now reflects real variations as soon as one has
    /// been created (Step 5, via `on_click_history`/`play_variation`); as long
    /// as none has been, the tree only contains a single line
    /// (depth 0 everywhere), identical to [`Self::build_move_rows`] but
    /// in a flattened form, node by node, rather than grouped by move
    /// pair.
    #[must_use]
    pub fn flatten_move_tree(&self) -> Vec<chess_core::game_tree::FlatNode> {
        self.game.history().tree().flatten()
    }

    // ── Context menu: NAG (PHASE 16, Step 6.1) ────────────────────────────────

    /// Applies the NAG annotation `code` (1 to 6, see
    /// [`chess_core::game_tree::Nag::from_code`] — context menu order,
    /// decision 7) to the move identified by `node_id`.
    ///
    /// Clicking the glyph already active for this move removes it (toggle)
    /// rather than reapplying it — avoids requiring a separate menu
    /// item "No annotation".
    ///
    /// Returns `true` if `node_id` exists and `code` is a valid NAG code
    /// (1 to 6), `false` otherwise (no change in that case).
    pub fn toggle_move_nag(&mut self, node_id: usize, code: i32) -> bool {
        let Ok(code_u8) = u8::try_from(code) else { return false; };
        let Some(nag) = chess_core::game_tree::Nag::from_code(code_u8) else { return false; };
        let Some(node) = self.game.history_mut().tree_mut().node_mut(node_id) else { return false; };

        node.nag = if node.nag == Some(nag) { None } else { Some(nag) };
        true
    }

    // ── Context menu: promote / remove a variation (PHASE 16, Step 6.2) ──────

    /// Promotes the variation starting at node `node_id` to the main line
    /// (decisions 2/7, right-click → "Promote to main line").
    ///
    /// Also moves the active line displayed in the main columns
    /// of the history up to the tip of the promoted variation — like playing
    /// a move from history (Step 5, [`Self::execute_variation_move`]):
    /// without this move, the old active line would end up displayed
    /// twice (see the docs of [`chess_core::history::History::promote_to_mainline`]).
    /// Resets the history view and current selection,
    /// and updates the last move highlighted on the board.
    ///
    /// Returns `true` if `node_id` exists.
    pub fn promote_variation_to_mainline(&mut self, node_id: usize) -> bool {
        let ok = self.game.history_mut().promote_to_mainline(node_id);
        if ok {
            // Bugfix PHASE 16 (04/07/2026): `promote_to_mainline` realigns
            // `path` (hence the active line) without going through `play`/`play_variation`
            // — `position`/`current_fen()` must be resynchronized
            // explicitly, otherwise they stay frozen on the old active line
            // (bug found by the Step 8 integration test).
            self.game.sync_position_with_history();
            self.last_move = self.game.history().last().map(|r| r.mv);
            self.viewed_ply = None;
            self.clear_selection();
        }
        ok
    }

    /// Removes the variation starting at node `node_id` (and its descendants)
    /// — decision 7, right-click → "Delete this variation".
    ///
    /// Never affects the active line or the displayed position (see the
    /// safeguard in [`chess_core::history::History::remove_variation`]):
    /// a variation is never itself the viewed line as long as no
    /// mechanism allows "viewing" a variation without playing it.
    ///
    /// Returns `true` if `node_id` exists and does not belong to the active
    /// line.
    pub fn remove_variation(&mut self, node_id: usize) -> bool {
        self.game.history_mut().remove_variation(node_id)
    }

    // ── Context menu: inline comment (PHASE 16, Step 6.3) ─────────────────────

    /// Sets (or clears) the comment of move `node_id` — decision 8,
    /// right-click → "Add a comment" (inline editing, never a
    /// modal window). Limited to the main line for this step
    /// (decision made on 04/07/2026): `node_id` is always a
    /// `white_node_id`/`black_node_id` from [`Self::build_move_rows`], never
    /// a `white_variation_node_id`/`black_variation_node_id`.
    ///
    /// An empty `text` clears the comment (stored as `None`) rather than
    /// as an empty string — avoids distinguishing "no comment" from
    /// "empty comment" elsewhere (`build_move_rows`, future PGN export).
    ///
    /// Returns `true` if `node_id` exists.
    pub fn set_move_comment(&mut self, node_id: usize, text: &str) -> bool {
        let Some(node) = self.game.history_mut().tree_mut().node_mut(node_id) else { return false; };
        node.comment = if text.is_empty() { None } else { Some(text.to_string()) };
        true
    }

    // ── Reset ──────────────────────────────────────────────────────────────────

    /// Resets the game to the initial position.
    pub fn reset(&mut self) {
        self.game                   = ChessGame::new();
        self.selection              = None;
        self.targets                = HashSet::new();
        self.last_move              = None;
        self.viewed_ply             = None;
        self.pending_promotion      = None;
        self.pending_promo_is_white = false;
        self.pending_promotion_ply  = None;
        // PHASE 26: a new game must never start in
        // variation-editing mode (a potential bug otherwise, discovered while
        // preparing Step 3 — `reset()` did not touch this field until now).
        self.variation_editing      = false;
    }

    // ── Undo ──────────────────────────────────────────────────────────────────

    /// Undoes the last move played.
    ///
    /// Resets the selection, targets, the last highlighted move,
    /// and leaves history-viewing mode if active.
    ///
    /// Returns `true` if a move was undone, `false` if the history is empty
    /// or if the internal FEN is corrupted (theoretical case, should not happen).
    pub fn undo_last_move(&mut self) -> bool {
        if self.viewed_ply.is_some() {
            // Leave viewing mode before undoing
            self.viewed_ply = None;
        }
        let undone = self.game.undo();
        if undone {
            self.selection              = None;
            self.targets                = HashSet::new();
            self.pending_promotion      = None;
            self.pending_promo_is_white = false;
            self.pending_promotion_ply  = None;
            // Last move = the second-to-last one in the history (if it exists)
            self.last_move = self.game.history().records().last().map(|r| r.mv);
        }
        undone
    }

    // ── PGN Export / Import ───────────────────────────────────────────────────

    /// Exports the current game to a PGN string (7 tags + moves in SAN).
    ///
    /// `white` and `black` are the player names recorded in the PGN tags.
    /// If the game is empty, the PGN result will contain `*`.
    #[must_use]
    pub fn export_pgn(&self, white: &str, black: &str) -> String {
        use chess_core::pgn::{export_pgn as core_export, PgnTags};
        let tags = PgnTags {
            event: "Vendetta Chess".into(),
            white: white.to_owned(),
            black: black.to_owned(),
            ..PgnTags::default()
        };
        core_export(&self.game, Some(tags))
    }

    /// Replaces the current game with the contents of a PGN file.
    ///
    /// On success, the GUI state (selection, history, promotion)
    /// is fully reset. The controller points to the final
    /// position of the imported game.
    ///
    /// # Errors
    ///
    /// Returns [`chess_core::pgn::PgnError`] if the PGN is invalid or empty.
    pub fn load_from_pgn(&mut self, pgn_str: &str) -> Result<(), chess_core::pgn::PgnError> {
        use chess_core::pgn::import_pgn as core_import;
        let new_game = core_import(pgn_str)?;
        self.game                   = new_game;
        self.selection              = None;
        self.targets                = HashSet::new();
        self.last_move              = None;
        self.viewed_ply             = None;
        self.pending_promotion      = None;
        self.pending_promo_is_white = false;
        self.pending_promotion_ply  = None;
        self.variation_editing      = false; // PHASE 26: see Self::reset
        Ok(())
    }

    // ── FEN loading ────────────────────────────────────────────────────────────

    /// Checks whether a FEN is syntactically valid, without modifying the game state.
    #[must_use]
    pub fn is_valid_fen(fen: &str) -> bool {
        ChessGame::from_fen(fen.trim()).is_ok()
    }

    /// Loads a position from a FEN, with no move history.
    ///
    /// The game restarts from the given position (no move recorded).
    /// Returns `true` if the FEN is valid and was loaded, `false` otherwise.
    pub fn load_from_fen(&mut self, fen: &str) -> bool {
        match ChessGame::from_fen(fen.trim()) {
            Ok(new_game) => {
                self.game                   = new_game;
                self.selection              = None;
                self.targets                = HashSet::new();
                self.last_move              = None;
                self.viewed_ply             = None;
                self.pending_promotion      = None;
                self.pending_promo_is_white = false;
                self.pending_promotion_ply  = None;
                self.variation_editing      = false; // PHASE 26: see Self::reset
                true
            }
            Err(_) => false,
        }
    }

    // ── Generating SquareData ─────────────────────────────────────────────────

    /// Builds the 64 [`SquareData`] from the current state (or the viewed ply).
    ///
    /// - At the current position: selection, legal targets and the last move are active.
    /// - In history mode: displays the position after the viewed ply + overlay
    ///   of the viewed move; selection and targets are disabled.
    #[must_use]
    pub fn build_squares(&self) -> Vec<SquareData> {
        // ── Position to display ─────────────────────────────────────────────────
        let pos: Position = if let Some(ply) = self.viewed_ply {
            self.game
                .position_at(ply + 1)
                .unwrap_or_else(|| self.game.position().clone())
        } else {
            self.game.position().clone()
        };

        // ── Overlay of the last move / viewed move ────────────────────────────
        let (last_from, last_to) = if let Some(ply) = self.viewed_ply {
            if let Some(record) = self.game.history().get(ply) {
                (Some(record.mv.from), Some(record.mv.to))
            } else {
                (None, None)
            }
        } else {
            (self.last_move.map(|m| m.from), self.last_move.map(|m| m.to))
        };

        // ── King in check (PHASE 67) ────────────────────────────────────────────
        // `is_in_check` always tests the side to move of `pos` — consistent
        // with the historical display: in `viewed_ply` mode, `pos` is already
        // the position AFTER this ply, so "to move" correctly designates the
        // side that just suffered the check at that point in the game, not the
        // actual current position.
        let check_square: Option<Square> = if is_in_check(&pos) {
            pos.board.find_king(pos.side_to_move)
        } else {
            None
        };

        // ── Checkmated king (PHASE 69, "groggy" effect) ─────────────────────────
        // `game_status` is only called if `check_square` is already known
        // (a checkmate is always a check) — avoids regenerating the
        // legal moves a second time in the common case (no check in progress).
        let mated_square: Option<Square> = if check_square.is_some()
            && game_status(&pos) == GameStatus::Checkmate
        {
            check_square
        } else {
            None
        };

        // ── Selection ──────────────────────────────────────────────────────────
        // PHASE 16, Step 5: `self.selection`/`self.targets` are no longer
        // systematically cleared in history mode — when variation
        // creation is active (`variation_editing`), an ongoing selection
        // must remain visible while a move is being chosen from a
        // past position. Outside this mode, `on_click_history` already
        // guarantees `selection` stays `None` (no call to `select_from` occurs).
        let active_selection = self.selection;
        let active_targets: &HashSet<Square> = &self.targets;

        // ── Assist mode (PHASE 68) ────────────────────────────────────────────
        // Computed only once for all target squares of the current
        // selection (not in the loop over the 64 squares below): negligible
        // cost (at most ~27 legal moves in the worst case, a queen in the
        // center of an empty board), no need to redo it per square.
        //
        // For each legal move from the selected square: simulates the move
        // (`make_move`, already used for legality — never a failure here
        // since `m` comes from `generate_legal_moves(&pos)`) then computes:
        //   - `is_capture_risk`: is the destination square attacked by the
        //     opposing side once the move is played (simple version agreed
        //     with the user — no material evaluation of the exchange);
        //   - `is_gives_check`/`is_gives_mate`: is the opposing side in
        //     check / checkmate in the resulting position
        //     (`chess_core::rules::game_status`, already used elsewhere for
        //     end-of-game detection — no logic reinvented here).
        //
        // Case of promotions (several legal moves then share the same
        // destination square, one per promotion piece): always simulated
        // with the Queen, regardless of the piece actually encoded in
        // `m` (choice agreed with the user). The Rook/Bishop/Knight variants
        // therefore produce exactly the same triple for this
        // square — no loss of information when collecting into the
        // `HashMap` below, which only keeps one entry per square.
        let assist_by_square: std::collections::HashMap<Square, (bool, bool, bool)> =
            match (self.assist_mode, active_selection) {
                (true, Some(from)) => generate_legal_moves(&pos)
                    .into_iter()
                    .filter(|m| m.from == from)
                    .map(|m| {
                        let sim_move = if m.promotion.is_some() {
                            Move { promotion: Some(PieceKind::Queen), ..m }
                        } else {
                            m
                        };
                        let flags = match make_move(&pos, sim_move) {
                            Ok(new_pos) => {
                                let gives_check = is_in_check(&new_pos);
                                let gives_mate = gives_check
                                    && game_status(&new_pos) == GameStatus::Checkmate;
                                let capture_risk = is_square_attacked(
                                    &new_pos.board, sim_move.to, new_pos.side_to_move,
                                );
                                (capture_risk, gives_check, gives_mate)
                            }
                            Err(_) => (false, false, false),
                        };
                        (m.to, flags)
                    })
                    .collect(),
                _ => std::collections::HashMap::new(),
            };

        // ── Building the vector ───────────────────────────────────────────────
        let mut squares = Vec::with_capacity(64);
        for row in 0..8_i32 {
            for col in 0..8_i32 {
                let sq    = slint_to_square(row, col);
                let piece = pos.board.piece_at(sq);

                let (piece_char, piece_side) = match piece {
                    Some(p) => (
                        slint::SharedString::from(piece_id(p)),
                        if p.color == Color::White { 1_i32 } else { 2_i32 },
                    ),
                    None => (slint::SharedString::from(""), 0_i32),
                };

                let (is_capture_risk, is_gives_check, is_gives_mate) =
                    assist_by_square.get(&sq).copied().unwrap_or((false, false, false));

                squares.push(SquareData {
                    row,
                    col,
                    piece_char,
                    is_light:        (row + col) % 2 == 0,
                    piece_side,
                    is_selected:     active_selection == Some(sq),
                    is_legal_target: active_targets.contains(&sq),
                    is_last_from:    last_from == Some(sq),
                    is_last_to:      last_to   == Some(sq),
                    is_king_in_check: check_square == Some(sq),
                    is_capture_risk,
                    is_gives_check,
                    is_gives_mate,
                    is_mated_king: mated_square == Some(sq),
                });
            }
        }
        squares
    }

    // ── Generating MoveRow ────────────────────────────────────────────────────

    /// PHASE 70 — category of a move for the syntax highlighting of the
    /// history panel (see `MoveRow.white_move_kind`/`black_move_kind`
    /// on the Slint side): decreasing priority mate > check > castle/promotion >
    /// capture > normal in case of overlap (e.g. a capture that also gives check
    /// is displayed as "check", not "capture" — decision made).
    ///
    ///   0 = normal · 1 = capture · 2 = castle · 3 = promotion
    ///   4 = check  · 5 = checkmate
    ///
    /// Reuses the SAN suffix already computed by `notation::move_to_san`
    /// ("+"/"#") to detect check/mate rather than replaying
    /// `movegen::is_in_check`, and the same capture detection as
    /// [`Self::captured_pieces`] (destination square already occupied before
    /// the move, or the opposing pawn's square for an en-passant capture).
    ///
    /// `pub` since 10/07/2026 (ergonomics follow-up): reused as-is
    /// by `main.rs::build_game_detail` to apply the same
    /// syntax highlighting to "Détail de la partie" — the same function, so
    /// guaranteed to stay identical to the main history panel.
    #[must_use]
    pub fn move_kind_code(record: &chess_core::history::MoveRecord) -> i32 {
        if record.san.ends_with('#') {
            return 5;
        }
        if record.san.ends_with('+') {
            return 4;
        }
        if record.mv.kind == MoveKind::Castle {
            return 2;
        }
        if record.mv.kind == MoveKind::Promotion {
            return 3;
        }
        let is_capture = record.mv.kind == MoveKind::EnPassant
            || Position::from_fen(&record.fen_before)
                .ok()
                .and_then(|pos| pos.board.piece_at(record.mv.to))
                .is_some();
        i32::from(is_capture)
    }

    /// Builds the list of [`MoveRow`] to display in the history panel.
    ///
    /// Each entry represents a full move (white + black if applicable).
    /// The number format is "1.", "2.", …
    ///
    /// PHASE 16, Step 4: each entry also carries the variations text
    /// (`white_variations`/`black_variations`) attached to this move, already
    /// formatted PGN-style by [`chess_core::game_tree::GameTree::build_variation_blocks`]
    /// (several variations for the same move joined by " | ", display
    /// only — not yet interactive, see `move_list.slint`). Empty as long
    /// as no variation has been created (Step 5, `on_click_history`).
    ///
    /// PHASE 16, Step 6.1: each entry also carries the node
    /// identifiers (`white_node_id`/`black_node_id`, `-1` if absent) needed for
    /// the context menu (right-click), the NAG glyph already applied to the
    /// main move (`white_nag`/`black_nag`, empty if none), and the identifier of
    /// the first move of the **first** variation at this ply
    /// (`white_variation_node_id`/`black_variation_node_id`, `-1` if none)
    /// — target of the right-click on the collapsed variation block (decision made:
    /// a collapsed block only exposes its starting move, not its individual
    /// internal moves; if several variations coexist at the same ply,
    /// only the first is targetable for now).
    ///
    /// PHASE 16, Step 6.3: each entry also carries the main move's free-text
    /// comment (`white_comment`/`black_comment`, empty string if
    /// none) — limited to the main line for this step (decision
    /// made on 04/07/2026), never populated for a variation node.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the internal `.expect("ply i existe")` calls
    /// operate on `0..total_plies` indices obtained from `history.len()`
    /// just before, so always present in the history.
    // Clippy: `#[allow(cast_possible_wrap)]` — ply indices and node
    // identifiers (`usize`) stay in practice very far from `i32::MAX`
    // (billions of moves/nodes), a safe conversion for the Slint display.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn build_move_rows(&self) -> Vec<MoveRow> {
        let history    = self.game.history();
        let total_plies = history.len();
        let mut rows   = Vec::with_capacity(total_plies.div_ceil(2));

        // (text, id of the first move of the variation), in the order of
        // `build_variation_blocks` — the first element per ply serves as the
        // right-click target (decision made, see doc above).
        let mut variations_by_ply: std::collections::HashMap<usize, Vec<(String, usize)>> =
            std::collections::HashMap::new();
        for block in history.tree().build_variation_blocks() {
            variations_by_ply.entry(block.after_ply).or_default().push((block.text, block.start_node_id));
        }
        let variation_text_for = |ply: usize| -> slint::SharedString {
            variations_by_ply
                .get(&ply)
                .map_or_else(slint::SharedString::default, |v| {
                    v.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join(" | ").into()
                })
        };
        let variation_node_id_for = |ply: usize| -> i32 {
            variations_by_ply
                .get(&ply)
                .and_then(|v| v.first())
                .map_or(-1_i32, |(_, id)| *id as i32)
        };
        let nag_symbol_for = |node_id: usize| -> slint::SharedString {
            history.tree()
                .node(node_id)
                .and_then(|n| n.nag)
                .map_or_else(slint::SharedString::default, |nag| nag.symbol().into())
        };
        // PHASE 16, Step 6.3: comment of the main move — limited to the
        // main line (never queried for a variation node here).
        let comment_for = |node_id: usize| -> slint::SharedString {
            history.tree()
                .node(node_id)
                .and_then(|n| n.comment.as_deref())
                .map_or_else(slint::SharedString::default, Into::into)
        };

        let mut i = 0_usize;
        while i < total_plies {
            let white_record = history.get(i).expect("ply i existe");
            let black_record = if i + 1 < total_plies { history.get(i + 1) } else { None };
            let white_node_id = history.node_id_at(i).expect("ply i existe");
            let black_node_id = if i + 1 < total_plies { history.node_id_at(i + 1) } else { None };

            rows.push(MoveRow {
                number_str: format!("{}.", i / 2 + 1).into(),
                white_san:  white_record.san.as_str().into(),
                black_san:  black_record.map_or(
                    slint::SharedString::default(),
                    |r| r.san.as_str().into(),
                ),
                white_ply:  i as i32,
                black_ply:  black_record.map_or(-1_i32, |_| (i + 1) as i32),
                white_from_book: white_record.from_book,
                black_from_book: black_record.is_some_and(|r| r.from_book),
                white_variations: variation_text_for(i),
                black_variations: black_record.map_or_else(
                    slint::SharedString::default,
                    |_| variation_text_for(i + 1),
                ),
                white_node_id: white_node_id as i32,
                black_node_id: black_node_id.map_or(-1_i32, |id| id as i32),
                white_nag: nag_symbol_for(white_node_id),
                black_nag: black_node_id.map_or_else(slint::SharedString::default, nag_symbol_for),
                white_variation_node_id: variation_node_id_for(i),
                black_variation_node_id: black_record.map_or(-1_i32, |_| variation_node_id_for(i + 1)),
                white_comment: comment_for(white_node_id),
                black_comment: black_node_id.map_or_else(slint::SharedString::default, comment_for),
                white_move_kind: Self::move_kind_code(white_record),
                black_move_kind: black_record.map_or(0, Self::move_kind_code),
            });

            i += 2;
        }
        rows
    }

    // ── Captured pieces ───────────────────────────────────────────────────────

    /// Computes the pieces captured since the start of the game (or up to the
    /// viewed ply, in history mode), grouped by the color of the captured
    /// piece: `captured_white` = white pieces taken (so taken BY
    /// Black), `captured_black` = black pieces taken (taken BY
    /// White). Each list is sorted by decreasing value.
    ///
    /// Capture detection identical to that of `movegen::apply_move`/
    /// `notation::move_to_san`: occupation of the destination square before
    /// the move, or the opposing pawn's square for an en passant.
    fn captured_pieces(&self) -> (Vec<PieceKind>, Vec<PieceKind>) {
        let limit = self.viewed_ply.map_or(usize::MAX, |p| p + 1);
        let mut captured_white = Vec::new();
        let mut captured_black = Vec::new();

        for record in self.game.history().records().iter().take(limit) {
            let Ok(pos_before) = Position::from_fen(&record.fen_before) else { continue };
            let captured = match record.mv.kind {
                MoveKind::EnPassant => {
                    let sq = Square::new(record.mv.to.file(), record.mv.from.rank());
                    pos_before.board.piece_at(sq)
                }
                _ => pos_before.board.piece_at(record.mv.to),
            };
            if let Some(p) = captured {
                match p.color {
                    Color::White => captured_white.push(p.kind),
                    Color::Black => captured_black.push(p.kind),
                }
            }
        }

        captured_white.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));
        captured_black.sort_by_key(|k| std::cmp::Reverse(piece_value(*k)));
        (captured_white, captured_black)
    }

    /// Compacts an already sorted list of `PieceKind` into (icon, count) pairs
    /// for the Slint display — groups consecutive occurrences of the same
    /// type (e.g. 2 pawns → one icon + count 2 rather than 2 icons).
    fn compact_captures(kinds: &[PieceKind], color: Color) -> Vec<CapturedPieceData> {
        let mut out: Vec<CapturedPieceData> = Vec::new();
        for &kind in kinds {
            let code = piece_id(Piece { color, kind });
            if let Some(last) = out.last_mut() {
                if last.piece_code.as_str() == code {
                    last.count += 1;
                    continue;
                }
            }
            out.push(CapturedPieceData {
                piece_code: slint::SharedString::from(code),
                count:      1,
            });
        }
        out
    }

    /// Captures summary ready for the Slint display.
    ///
    /// Returns `(White's trophies, Black's trophies, differential)`:
    /// - White's trophies = black pieces White has captured;
    /// - Black's trophies = white pieces Black has captured;
    /// - differential     = material balance from White's point of view
    ///   (positive = white advantage, negative = black advantage), in points
    ///   (P=1, N/B=3, R=5, Q=9).
    #[must_use]
    pub fn captured_summary(&self) -> (Vec<CapturedPieceData>, Vec<CapturedPieceData>, i32) {
        let (captured_white, captured_black) = self.captured_pieces();

        let white_material_lost: i32 = captured_white.iter().copied().map(piece_value).sum();
        let black_material_lost: i32 = captured_black.iter().copied().map(piece_value).sum();

        let white_trophies = Self::compact_captures(&captured_black, Color::Black);
        let black_trophies = Self::compact_captures(&captured_white, Color::White);
        let diff = black_material_lost - white_material_lost;

        (white_trophies, black_trophies, diff)
    }

    // ── Status ────────────────────────────────────────────────────────────────

    /// i18n key for the status bar (always based on the actual game,
    /// not on the viewed ply).
    #[must_use]
    pub fn status_key(&self) -> &'static str {
        match self.game.result {
            GameResult::WhiteWins => "game.result.white_wins",
            GameResult::BlackWins => "game.result.black_wins",
            GameResult::Draw      => "game.result.draw",
            GameResult::Ongoing   => match self.game.position().side_to_move {
                Color::White => "board.turn.white",
                Color::Black => "board.turn.black",
            },
        }
    }

    /// i18n key for the **reason** the game ended (displayed as a subtitle
    /// on the game-over banner).
    ///
    /// Returns `""` if the game is still in progress.
    #[must_use]
    pub fn end_reason_key(&self) -> &'static str {
        match self.game.result {
            GameResult::WhiteWins | GameResult::BlackWins => "board.checkmate",
            GameResult::Draw => {
                // Stalemate = draw + no legal move for the side to move
                let legal = generate_legal_moves(self.game.position());
                if legal.is_empty() { "board.stalemate" } else { "board.draw" }
            }
            GameResult::Ongoing => "",
        }
    }

    /// `true` if it's White's turn (and the game is not over).
    #[must_use]
    pub fn is_white_turn(&self) -> bool {
        !self.game.is_over() && self.game.position().side_to_move == Color::White
    }

    /// FEN of the current position (for engine analysis).
    #[must_use]
    pub fn current_fen(&self) -> String {
        self.game.position().to_fen()
    }

    /// FEN of the position **displayed** on screen (respects `viewed_ply`),
    /// unlike [`Self::current_fen`] which always reflects the actual
    /// position of the game.
    ///
    /// Used by the board's PNG export (PHASE 76), designed as a
    /// "snapshot" of what the user is looking at the moment of the click, including
    /// while navigating through the history — a different decision from
    /// the PDF export (always the final position, see `pdf_export.rs`).
    /// Same position-selection logic as [`Self::build_squares`].
    #[must_use]
    pub fn displayed_fen(&self) -> String {
        let pos: Position = if let Some(ply) = self.viewed_ply {
            self.game
                .position_at(ply + 1)
                .unwrap_or_else(|| self.game.position().clone())
        } else {
            self.game.position().clone()
        };
        pos.to_fen()
    }

    /// SAN notation of the last move played, or `None` if the history is empty.
    ///
    /// Used for the "book move played" notification shown in the UI
    /// (PHASE 15, Step 6bis) — called right after [`Self::apply_uci_move_from_book`]
    /// to display the move actually played rather than its raw UCI notation.
    #[must_use]
    pub fn last_move_san(&self) -> Option<String> {
        self.game.history().last().map(|r| r.san.clone())
    }

    /// UCI notation of the last move played, or `None` if the history is empty.
    ///
    /// Used in Puzzle mode (PHASE 14, Step 6) to validate the move the
    /// user just made via [`Self::on_click`]/
    /// [`Self::complete_promotion`] against `PuzzleSession::try_move_uci`,
    /// without having to duplicate the two-click selection state machine.
    #[must_use]
    pub fn last_move_uci(&self) -> Option<String> {
        self.last_move.map(Move::to_uci)
    }

    /// Applies a move in UCI notation (e.g. `"e2e4"`, `"e7e8q"`).
    ///
    /// Used by the engine-player bridge ([`crate::game_bridge::GameBridge`]) to
    /// apply the `bestmove` received from the engine without going through the
    /// two-click selection machinery.
    ///
    /// - Promotion not specified in the string → Queen by default.
    /// - Returns `true` if the move was played successfully.
    pub fn apply_uci_move(&mut self, uci: &str) -> bool {
        self.apply_uci_move_impl(uci, false)
    }

    /// Applies a move in UCI notation, marking it as coming from a
    /// Polyglot opening book (PHASE 15) rather than an engine computation.
    ///
    /// Identical to [`Self::apply_uci_move`] except for this metadata
    /// (📖 icon in the move list, see `MoveRow`/`move_list.slint`).
    pub fn apply_uci_move_from_book(&mut self, uci: &str) -> bool {
        self.apply_uci_move_impl(uci, true)
    }

    fn apply_uci_move_impl(&mut self, uci: &str, from_book: bool) -> bool {
        // Special case of the UCI protocol: `bestmove (none)` is returned by an
        // engine when no move is legal (position already checkmate/stalemate).
        // Explicitly rejected here for clarity — the byte-level parsing
        // below would reject it anyway (out-of-bounds wrapping_sub on
        // `(`), but an explicit rejection documents the intent.
        if uci == "(none)" || uci.is_empty() { return false; }

        // Minimal format: 4 bytes (e.g. "e2e4")
        let b = uci.as_bytes();
        if b.len() < 4 { return false; }

        let from_file = b[0].wrapping_sub(b'a');
        let from_rank = b[1].wrapping_sub(b'1');
        let to_file   = b[2].wrapping_sub(b'a');
        let to_rank   = b[3].wrapping_sub(b'1');

        if from_file > 7 || from_rank > 7 || to_file > 7 || to_rank > 7 { return false; }

        let from = Square::new(from_file, from_rank);
        let to   = Square::new(to_file,   to_rank);

        // Promotion piece (optional 5th character: q / r / b / n)
        let promo_kind: Option<PieceKind> = if b.len() >= 5 {
            match b[4].to_ascii_lowercase() {
                b'q' => Some(PieceKind::Queen),
                b'r' => Some(PieceKind::Rook),
                b'b' => Some(PieceKind::Bishop),
                b'n' => Some(PieceKind::Knight),
                _    => None,
            }
        } else {
            None
        };

        let legal = generate_legal_moves(self.game.position());

        // If the move is a promotion with no piece specified → Queen by default.
        let promo = if promo_kind.is_none()
            && legal.iter().any(|m| m.from == from && m.to == to && m.promotion.is_some())
        {
            Some(PieceKind::Queen)
        } else {
            promo_kind
        };

        let mv = legal.iter()
            .find(|m| m.from == from && m.to == to && m.promotion == promo)
            .copied();

        if let Some(mv) = mv {
            // Code audit 04/07/2026, point 4: see the equivalent comment
            // in `execute_variation_move` — `mv` comes from `generate_legal_moves`,
            // should never fail to play.
            let result = self.game.play(mv);
            debug_assert!(result.is_ok(), "coup légal qui échoue à jouer : {mv:?}");
            if from_book {
                self.game.mark_last_move_as_book();
            }
            self.last_move  = Some(mv);
            self.viewed_ply = None; // leave history mode if active
            crate::debug_log::log_event("move_played", &serde_json::json!({
                "source": if from_book { "book" } else { "engine" },
                "from": from.to_algebraic(),
                "to": to.to_algebraic(),
                "san": self.last_move_san(),
                "uci": mv.to_uci(),
                "move_count": self.game.move_count(),
            }));
            self.clear_selection();
            true
        } else {
            false
        }
    }

    /// Number of half-moves played since the start.
    #[must_use]
    pub fn move_count(&self) -> usize {
        self.game.move_count()
    }

    /// `true` if the game is over.
    #[must_use]
    pub fn is_over(&self) -> bool {
        self.game.is_over()
    }

    /// PGN result of the finished game: `"1-0"`, `"0-1"` or `"1/2-1/2"`.
    ///
    /// Returns `None` if the game is still in progress.
    #[must_use]
    pub fn result_pgn(&self) -> Option<&'static str> {
        match self.game.result {
            GameResult::WhiteWins => Some("1-0"),
            GameResult::BlackWins => Some("0-1"),
            GameResult::Draw      => Some("1/2-1/2"),
            GameResult::Ongoing   => None,
        }
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Selects `sq` and computes the legal targets.
    fn select(&mut self, sq: Square) {
        self.selection = Some(sq);
        let legal = generate_legal_moves(self.game.position());
        self.targets = legal.iter()
            .filter(|m| m.from == sq)
            .map(|m| m.to)
            .collect();
    }

    /// Clears the current selection.
    fn clear_selection(&mut self) {
        self.selection = None;
        self.targets.clear();
    }

    /// Plays the move `from → to`.
    ///
    /// If moves with promotion exist for this trajectory, sets
    /// `pending_promotion` and returns `false` (the move is not yet played).
    /// Otherwise plays the move immediately and returns `true`.
    fn execute_move(&mut self, from: Square, to: Square) -> bool {
        let legal = generate_legal_moves(self.game.position());
        let candidates: Vec<Move> = legal.iter()
            .filter(|m| m.from == from && m.to == to)
            .copied()
            .collect();

        // Promotion detection: several moves exist (one per piece)
        // OR a single move with `promotion.is_some()`.
        let is_promotion = candidates.iter().any(|m| m.promotion.is_some());

        if is_promotion {
            self.pending_promotion      = Some((from, to));
            self.pending_promo_is_white = self.game.position().side_to_move == Color::White;
            return false;
        }

        // Normal move (including en passant, castling)
        if let Some(&mv) = candidates.first() {
            // Code audit 04/07/2026, point 4: see the equivalent comment
            // in `execute_variation_move` — `mv` comes from `generate_legal_moves`,
            // should never fail to play.
            let result = self.game.play(mv);
            debug_assert!(result.is_ok(), "coup légal qui échoue à jouer : {mv:?}");
            self.last_move = Some(mv);
            crate::debug_log::log_event("move_played", &serde_json::json!({
                "source": "human_live",
                "from": from.to_algebraic(),
                "to": to.to_algebraic(),
                "san": self.last_move_san(),
                "uci": mv.to_uci(),
                "move_count": self.game.move_count(),
            }));
            self.clear_selection();
            return true;
        }

        false
    }

    // ── History mode with variation creation (PHASE 16, Step 5) ──────────────

    /// Selects `sq` and computes the legal targets from `pos` rather than
    /// from the current position — counterpart of [`Self::select`] for
    /// history mode with active variation creation.
    fn select_from(&mut self, pos: &Position, sq: Square) {
        self.selection = Some(sq);
        let legal = generate_legal_moves(pos);
        self.targets = legal.iter()
            .filter(|m| m.from == sq)
            .map(|m| m.to)
            .collect();
    }

    /// Plays the move `from → to` as a variation from the viewed ply `ply`
    /// — counterpart of [`Self::execute_move`] for history mode with
    /// active variation creation.
    ///
    /// If the move is played successfully, the controller leaves
    /// history mode and ends up at the tip of the new variation:
    /// exactly as at the current position, further moves can then be
    /// chained, naturally extending this freshly
    /// created variation (see [`chess_core::history::History::branch_at`]).
    fn execute_variation_move(&mut self, ply: usize, from: Square, to: Square) -> bool {
        let pos = self.game
            .position_at(ply + 1)
            .unwrap_or_else(|| self.game.position().clone());
        let legal = generate_legal_moves(&pos);
        let candidates: Vec<Move> = legal.iter()
            .filter(|m| m.from == from && m.to == to)
            .copied()
            .collect();

        crate::debug_log::log_event("execute_variation_move_attempt", &serde_json::json!({
            "ply": ply,
            "from": from.to_algebraic(),
            "to": to.to_algebraic(),
            "side_to_move": format!("{:?}", pos.side_to_move),
            "nb_candidats": candidates.len(),
        }));

        let is_promotion = candidates.iter().any(|m| m.promotion.is_some());
        if is_promotion {
            crate::debug_log::log_event("execute_variation_move_promotion_deferred", &serde_json::json!({
                "from": from.to_algebraic(),
                "to": to.to_algebraic(),
                "ply": ply,
            }));
            self.pending_promotion      = Some((from, to));
            self.pending_promo_is_white = pos.side_to_move == Color::White;
            self.pending_promotion_ply  = Some(ply);
            return false;
        }

        if let Some(&mv) = candidates.first() {
            if self.game.play_variation(ply, mv).is_ok() {
                crate::debug_log::log_event("move_played", &serde_json::json!({
                    "source": "variation",
                    "from": from.to_algebraic(),
                    "to": to.to_algebraic(),
                    "san": self.last_move_san(),
                    "uci": mv.to_uci(),
                    "move_count": self.game.move_count(),
                }));
                self.last_move  = Some(mv);
                self.viewed_ply = None;
                self.clear_selection();
                return true;
            }
        }

        crate::debug_log::log_event("execute_variation_move_failed", &serde_json::json!({
            "from": from.to_algebraic(),
            "to": to.to_algebraic(),
            "ply": ply,
        }));
        false
    }
}

impl Default for GameController {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Coordinate conversions ───────────────────────────────────────────────

    #[test]
    fn test_slint_to_square_e1() {
        let sq = slint_to_square(7, 4);
        assert_eq!(sq.to_algebraic(), "e1");
    }

    #[test]
    fn test_slint_to_square_e8() {
        let sq = slint_to_square(0, 4);
        assert_eq!(sq.to_algebraic(), "e8");
    }

    #[test]
    fn test_slint_to_square_a1() {
        let sq = slint_to_square(7, 0);
        assert_eq!(sq.to_algebraic(), "a1");
    }

    #[test]
    fn test_slint_to_square_h8() {
        let sq = slint_to_square(0, 7);
        assert_eq!(sq.to_algebraic(), "h8");
    }

    // ── Initial state ─────────────────────────────────────────────────────────

    #[test]
    fn test_new_has_32_pieces() {
        let ctrl = GameController::new();
        let squares = ctrl.build_squares();
        assert_eq!(squares.iter().filter(|s| s.piece_side != 0).count(), 32);
    }

    #[test]
    fn test_initial_no_selection() {
        let ctrl = GameController::new();
        let squares = ctrl.build_squares();
        assert!(!squares.iter().any(|s| s.is_selected));
    }

    #[test]
    fn test_is_white_turn_initially() {
        assert!(GameController::new().is_white_turn());
    }

    #[test]
    fn test_status_key_initial() {
        assert_eq!(GameController::new().status_key(), "board.turn.white");
    }

    // ── Selection ─────────────────────────────────────────────────────────────

    #[test]
    fn test_click_e2_selects_pawn() {
        let mut ctrl = GameController::new();
        assert!(ctrl.on_click(6, 4));
        let e2 = ctrl.build_squares().into_iter()
            .find(|s| s.row == 6 && s.col == 4).unwrap();
        assert!(e2.is_selected);
    }

    #[test]
    fn test_e2_has_two_legal_targets() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4);
        let targets: Vec<_> = ctrl.build_squares().into_iter()
            .filter(|s| s.is_legal_target).collect();
        assert_eq!(targets.len(), 2, "e2 → e3 ou e4");
    }

    #[test]
    fn test_click_same_square_deselects() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4);
        ctrl.on_click(6, 4);
        assert!(!ctrl.build_squares().iter().any(|s| s.is_selected));
    }

    #[test]
    fn test_click_empty_square_no_change() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.on_click(4, 4));
    }

    // ── Executing a move ──────────────────────────────────────────────────────

    #[test]
    fn test_play_e2e4() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4);
        ctrl.on_click(4, 4);
        let squares = ctrl.build_squares();
        let e4 = squares.iter().find(|s| s.row == 4 && s.col == 4).unwrap();
        assert_eq!(e4.piece_char.as_str(), "wP");
        assert!(e4.is_last_to);
        let e2 = squares.iter().find(|s| s.row == 6 && s.col == 4).unwrap();
        assert!(e2.is_last_from);
        assert!(!e2.is_selected);
    }

    #[test]
    fn test_after_one_move_black_turn() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4);
        ctrl.on_click(4, 4);
        assert!(!ctrl.is_white_turn());
        assert_eq!(ctrl.status_key(), "board.turn.black");
    }

    #[test]
    fn test_change_selection_mid_turn() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 3);
        ctrl.on_click(6, 4);
        let squares = ctrl.build_squares();
        let d2 = squares.iter().find(|s| s.row == 6 && s.col == 3).unwrap();
        let e2 = squares.iter().find(|s| s.row == 6 && s.col == 4).unwrap();
        assert!(!d2.is_selected);
        assert!(e2.is_selected);
    }

    // ── build_move_rows ───────────────────────────────────────────────────────

    #[test]
    fn test_build_move_rows_empty() {
        assert!(GameController::new().build_move_rows().is_empty());
    }

    #[test]
    fn test_build_move_rows_after_e4() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4);
        ctrl.on_click(4, 4);
        let rows = ctrl.build_move_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number_str.as_str(), "1.");
        assert_eq!(rows[0].white_san.as_str(), "e4");
        assert_eq!(rows[0].black_san.as_str(), "");
        assert_eq!(rows[0].white_ply, 0);
        assert_eq!(rows[0].black_ply, -1);
        // No mechanism yet plays a move from a book (Step 6,
        // not wired up) — the flag must stay false for any game played
        // normally, including for the missing black cell (black_ply = -1).
        assert!(!rows[0].white_from_book);
        assert!(!rows[0].black_from_book);
    }

    #[test]
    fn test_build_move_rows_after_e4_e5() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.on_click(1, 4); ctrl.on_click(3, 4);
        let rows = ctrl.build_move_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white_san.as_str(), "e4");
        assert_eq!(rows[0].black_san.as_str(), "e5");
        assert_eq!(rows[0].white_ply, 0);
        assert_eq!(rows[0].black_ply, 1);
        assert!(!rows[0].white_from_book);
        assert!(!rows[0].black_from_book);
    }

    #[test]
    fn test_build_move_rows_two_full_moves() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.on_click(1, 4); ctrl.on_click(3, 4);
        ctrl.on_click(7, 6); ctrl.on_click(5, 5);
        let rows = ctrl.build_move_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].number_str.as_str(), "2.");
        assert_eq!(rows[1].white_san.as_str(), "Nf3");
        assert_eq!(rows[1].black_san.as_str(), "");
        assert_eq!(rows[1].white_ply, 2);
        assert_eq!(rows[1].black_ply, -1);
    }

    // ── build_move_rows: variations (PHASE 16, Step 4) ───────────────────────
    // Non-regression: with no variation created (no branch_at, Step 5),
    // these fields must always stay empty.

    #[test]
    fn test_build_move_rows_variations_empty_without_branching() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.on_click(1, 4); ctrl.on_click(3, 4);

        let rows = ctrl.build_move_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white_variations.as_str(), "");
        assert_eq!(rows[0].black_variations.as_str(), "");
    }

    // ── go_to_ply ─────────────────────────────────────────────────────────────

    #[test]
    fn test_go_to_ply_returns_false_when_no_history() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.go_to_ply(0));
    }

    #[test]
    fn test_go_to_ply_shows_historical_position() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.on_click(1, 4); ctrl.on_click(3, 4);
        assert!(ctrl.go_to_ply(0));
        let squares = ctrl.build_squares();
        let e4 = squares.iter().find(|s| s.row == 4 && s.col == 4).unwrap();
        assert_eq!(e4.piece_char.as_str(), "wP");
        let e5 = squares.iter().find(|s| s.row == 3 && s.col == 4).unwrap();
        assert_eq!(e5.piece_char.as_str(), "");
    }

    #[test]
    fn test_go_to_ply_minus1_returns_current() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.go_to_ply(0);
        assert!(ctrl.go_to_ply(-1));
        assert_eq!(ctrl.viewed_ply_slint(), -1);
    }

    #[test]
    fn test_click_while_in_history_mode_returns_current() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        ctrl.go_to_ply(0);
        assert!(ctrl.on_click(3, 3));
        assert_eq!(ctrl.viewed_ply_slint(), -1);
    }

    #[test]
    fn test_viewed_ply_slint_initial() {
        assert_eq!(GameController::new().viewed_ply_slint(), -1);
    }

    // ── Creating a variation in history mode (PHASE 16, Step 5) ─────────────

    #[test]
    fn test_variation_mode_disabled_by_default() {
        assert!(!GameController::new().variation_editing);
    }

    // ── Explicit variation-editing mode (PHASE 26, Step 1) ────────────────────

    #[test]
    fn test_is_variation_editing_false_by_default() {
        assert!(!GameController::new().is_variation_editing());
    }

    #[test]
    fn test_enter_variation_editing_fails_without_viewed_ply() {
        // Nothing to view (current position) → no variation possible.
        let mut ctrl = GameController::new();
        assert!(!ctrl.enter_variation_editing());
        assert!(!ctrl.is_variation_editing());
    }

    #[test]
    fn test_enter_variation_editing_succeeds_when_viewing_history() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0); // view after 1.e4

        assert!(ctrl.enter_variation_editing());
        assert!(ctrl.is_variation_editing());
    }

    #[test]
    fn test_enter_variation_editing_clears_any_residual_selection() {
        // Bugfix (user feedback 04/07/2026): a residual
        // selection (a piece already selected before entering editing
        // mode, whatever its exact origin) must never
        // survive `enter_variation_editing()` — otherwise the very first
        // click on the board that follows would be interpreted as a click on
        // a target square for this residual piece, playing a move without
        // the user's knowledge rather than simply selecting the
        // piece actually clicked.
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0); // view after 1.e4

        // Simulates a residual selection (regardless of how it would have
        // arisen in practice).
        ctrl.selection = Some(slint_to_square(7, 4));                  // e1, the white king
        ctrl.targets    = [slint_to_square(6, 4)].into_iter().collect(); // arbitrary target: e2

        assert!(ctrl.enter_variation_editing());
        assert!(ctrl.selection.is_none(), "toute sélection résiduelle doit être purgée à l'entrée en mode édition");
        assert!(ctrl.targets.is_empty());
    }

    #[test]
    fn test_exit_variation_editing_resets_flag_and_viewed_ply() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.enter_variation_editing();

        ctrl.exit_variation_editing();
        assert!(!ctrl.is_variation_editing());
        assert_eq!(ctrl.viewed_ply_slint(), -1, "retour à la position courante");
    }

    #[test]
    fn test_exit_variation_editing_safe_when_not_active() {
        // Must never panic nor change state unexpectedly,
        // even if the mode was not active.
        let mut ctrl = GameController::new();
        ctrl.exit_variation_editing();
        assert!(!ctrl.is_variation_editing());
        assert_eq!(ctrl.viewed_ply_slint(), -1);
    }

    #[test]
    fn test_variation_editing_persists_across_moves_unlike_viewed_ply() {
        // The core of the PHASE 26 bugfix: unlike `viewed_ply` (which
        // goes back to -1 as soon as the first move is played in the variation, see
        // `execute_variation_move`), `is_variation_editing()` must stay
        // true until `exit_variation_editing()` has been called
        // explicitly — allowing several moves to be chained within the
        // same editing session without having to click back on the history.
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0); // return after 1.e4
        assert!(ctrl.enter_variation_editing());

        // Plays 1...c5 instead of 1...e5 → creates a variation.
        assert!(ctrl.on_click(1, 2) && ctrl.on_click(3, 2));
        assert_eq!(ctrl.viewed_ply_slint(), -1, "viewed_ply repasse à -1 après le coup");
        assert!(ctrl.is_variation_editing(), "le mode d'édition, lui, doit rester actif");

        // Play can continue normally in the new line.
        assert!(ctrl.on_click(7, 6) && ctrl.on_click(5, 5)); // 2.Nf3
        assert_eq!(ctrl.move_count(), 3);

        ctrl.exit_variation_editing();
        assert!(!ctrl.is_variation_editing());
    }

    #[test]
    fn test_history_click_without_variation_mode_returns_current() {
        // Historical behavior preserved as long as `variation_editing`
        // is not explicitly enabled by the GUI.
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0); // view after 1.e4 (black to move)

        // Clicking the black pawn e7: without variation_editing, simply
        // returns to the current position rather than selecting it.
        assert!(ctrl.on_click(1, 4));
        assert_eq!(ctrl.viewed_ply_slint(), -1);
        assert!(!ctrl.build_squares().iter().any(|s| s.is_selected));
    }

    #[test]
    fn test_variation_mode_click_selects_piece_at_viewed_position() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.set_variation_mode_enabled(true);

        // Select the black pawn c7 (row1, col2) on the viewed position.
        assert!(ctrl.on_click(1, 2));
        assert_eq!(ctrl.viewed_ply_slint(), 0, "reste en mode historique");
        let squares = ctrl.build_squares();
        let c7 = squares.iter().find(|s| s.row == 1 && s.col == 2).unwrap();
        assert!(c7.is_selected);
        assert!(squares.iter().any(|s| s.is_legal_target), "des cibles légales doivent être calculées");
    }

    #[test]
    fn test_variation_mode_play_move_creates_variation_without_deleting_mainline() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0); // view after 1.e4
        ctrl.set_variation_mode_enabled(true);

        ctrl.on_click(1, 2); // select c7
        assert!(ctrl.on_click(3, 2)); // play c7-c5 → creates a variation

        // Automatic return to the current position, at the tip of the
        // new variation.
        assert_eq!(ctrl.viewed_ply_slint(), -1);
        assert_eq!(ctrl.move_count(), 2);

        let rows = ctrl.build_move_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].white_san.as_str(), "e4");
        assert_eq!(rows[0].black_san.as_str(), "c5", "la ligne active reflète le coup joué");

        // The old continuation (1...e5) is not lost: it appears as a
        // variation, without duplicating c5.
        assert!(rows[0].black_variations.as_str().contains("e5"));
        assert!(!rows[0].black_variations.as_str().contains("c5"));
    }

    #[test]
    fn test_variation_mode_reclick_same_square_deselects_without_exiting_history() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.set_variation_mode_enabled(true);

        ctrl.on_click(1, 2); // select c7
        assert!(ctrl.on_click(1, 2)); // click c7 again → deselect
        assert_eq!(ctrl.viewed_ply_slint(), 0, "toujours en mode historique");
        assert!(!ctrl.build_squares().iter().any(|s| s.is_selected));
    }

    #[test]
    fn test_variation_mode_irrelevant_click_stays_on_viewed_ply() {
        // Bugfix (user feedback 04/07/2026, real bug confirmed by
        // the diagnostic log): unlike the behavior before
        // PHASE 26 (locked in until now by this test under its former name
        // `test_variation_mode_irrelevant_click_returns_to_current`), an irrelevant click
        // WHILE editing a variation must no longer ever
        // silently return to the current position — only explicit
        // exit (`exit_variation_editing`) should. Otherwise the
        // "End the variation" banner stays shown (`variation_editing` still
        // true) while subsequent clicks would in fact be handled
        // against the live current position, potentially playing a real
        // game move without the user's knowledge.
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.set_variation_mode_enabled(true);

        // Unrelated empty square, no active selection.
        assert!(!ctrl.on_click(4, 4), "aucun changement d'état, rien à re-rendre");
        assert_eq!(ctrl.viewed_ply_slint(), 0, "doit rester sur le ply visualisé");
        assert!(ctrl.is_variation_editing(), "le mode d'édition reste actif");
    }

    #[test]
    fn test_variation_mode_wrong_color_click_stays_on_viewed_ply_with_selection_active() {
        // Same bugfix as above, but with a selection already active:
        // clicking a piece of the wrong color (thus irrelevant
        // to the ongoing selection) must only deselect the piece, neither
        // exit editing nor change the viewed ply.
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0); // view after 1.e4 — Black to move
        ctrl.set_variation_mode_enabled(true);

        ctrl.on_click(0, 1); // select the black knight b8 (right color here)
        assert!(ctrl.selection.is_some());

        // Click on a white piece (queen d1): wrong color for
        // the viewed position (Black to move), and not a legal target for
        // the knight b8.
        assert!(ctrl.on_click(7, 3));
        assert_eq!(ctrl.viewed_ply_slint(), 0, "doit rester sur le ply visualisé");
        assert!(ctrl.is_variation_editing(), "le mode d'édition reste actif");
        assert!(ctrl.selection.is_none(), "la sélection non pertinente est abandonnée");
    }

    #[test]
    fn test_variation_mode_promotion_deferred_then_completed() {
        // White pawn on e7, black king on a8, white king on e1 — same
        // position as `test_pending_promotion_detected`, but reached here
        // in history mode after playing a neutral move.
        let fen = "k7/4P3/8/8/8/8/8/4K3 w - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection: None, targets: HashSet::new(), last_move: None,
            viewed_ply: None, pending_promotion: None, pending_promo_is_white: false,
            pending_promotion_ply: None, variation_editing: false, assist_mode: false,
        };

        // Play two neutral king moves (white then black), so it's then
        // possible to view a historical position where WHITE is once
        // again to move (needed to select the e7 pawn).
        ctrl.on_click(7, 4); ctrl.on_click(7, 3); // Ke1-d1 (ply 0)
        ctrl.on_click(0, 0); ctrl.on_click(0, 1); // Ka8-b8 (ply 1)
        assert_eq!(ctrl.move_count(), 2);

        ctrl.go_to_ply(1); // re-view after Kb8: White to move
        ctrl.set_variation_mode_enabled(true);

        ctrl.on_click(1, 4); // select the e7 pawn
        assert!(ctrl.on_click(0, 4)); // e7-e8 → promotion pending
        assert!(ctrl.has_pending_promotion());
        assert_eq!(ctrl.move_count(), 2, "coup pas encore joué");

        let ok = ctrl.complete_promotion(1); // 1 = Queen
        assert!(ok);
        assert!(!ctrl.has_pending_promotion());
        assert_eq!(ctrl.viewed_ply_slint(), -1, "retour à la position courante après la variante");
        assert_eq!(ctrl.move_count(), 3, "Kd1, Kb8, puis e8=Q en variante");

        let squares = ctrl.build_squares();
        let e8 = squares.iter().find(|s| s.row == 0 && s.col == 4).unwrap();
        assert_eq!(e8.piece_char.as_str(), "wQ");
    }

    // ── Context menu: NAG (PHASE 16, Step 6.1) ────────────────────────────────

    #[test]
    fn test_toggle_move_nag_sets_and_reflects_in_build_move_rows() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4

        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;
        assert!(ctrl.toggle_move_nag(node_id, 3)); // code 3 = "!!"

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_nag.as_str(), "!!");
    }

    #[test]
    fn test_toggle_move_nag_reclick_same_code_removes_it() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);

        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;
        assert!(ctrl.toggle_move_nag(node_id, 3));
        assert!(ctrl.toggle_move_nag(node_id, 3)); // same code → toggles, removes it

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_nag.as_str(), "");
    }

    #[test]
    fn test_toggle_move_nag_different_code_replaces_previous() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);

        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;
        assert!(ctrl.toggle_move_nag(node_id, 3)); // "!!"
        assert!(ctrl.toggle_move_nag(node_id, 2)); // "?" — replaces (different codes)

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_nag.as_str(), "?");
    }

    #[test]
    fn test_toggle_move_nag_invalid_code_returns_false() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;

        assert!(!ctrl.toggle_move_nag(node_id, 0));
        assert!(!ctrl.toggle_move_nag(node_id, 7));
        assert!(!ctrl.toggle_move_nag(node_id, -1));
        assert_eq!(ctrl.build_move_rows()[0].white_nag.as_str(), "", "aucune modification");
    }

    #[test]
    fn test_toggle_move_nag_unknown_node_id_returns_false() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        assert!(!ctrl.toggle_move_nag(9999, 1));
    }

    #[test]
    fn test_build_move_rows_variation_node_id_targets_demoted_continuation() {
        // Reuses the scenario from `test_variation_mode_play_move_creates_variation_without_deleting_mainline`:
        // 1...e5 is demoted to a variation by 1...c5 — the node targeted by a
        // right-click on this collapsed block must be the one for e5 (not c5).
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0);
        ctrl.set_variation_mode_enabled(true);
        ctrl.on_click(1, 2); ctrl.on_click(3, 2); // 1...c5 → variation

        let rows = ctrl.build_move_rows();
        assert!(rows[0].black_variation_node_id >= 0);

        assert!(ctrl.toggle_move_nag(rows[0].black_variation_node_id as usize, 2)); // "?"
        let rows2 = ctrl.build_move_rows();
        assert!(
            rows2[0].black_variations.as_str().contains("e5?"),
            "le NAG doit apparaître collé au SAN dans le texte replié : {}",
            rows2[0].black_variations.as_str()
        );
    }

    // ── Context menu: promote / remove a variation (PHASE 16, Step 6.2) ──────

    #[test]
    // Clippy: `#[allow(cast_possible_wrap)]` — a test node identifier,
    // very far from `i32::MAX`.
    #[allow(clippy::cast_possible_wrap)]
    fn test_promote_variation_to_mainline_realigns_active_line() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.on_click(7, 6); ctrl.on_click(5, 5); // 2.Nf3
        ctrl.on_click(0, 1); ctrl.on_click(2, 2); // 2...Nc6
        let nf3_id = ctrl.build_move_rows()[1].white_node_id as usize;

        ctrl.go_to_ply(1); // move back to after 1...e5
        ctrl.set_variation_mode_enabled(true);
        ctrl.on_click(7, 1); ctrl.on_click(5, 2); // 2.Nc3 (variation) → Nf3 demoted

        let rows = ctrl.build_move_rows();
        assert_eq!(
            rows[1].white_variation_node_id, nf3_id as i32,
            "Nf3 doit apparaître comme variante après la création de 2.Nc3"
        );

        assert!(ctrl.promote_variation_to_mainline(nf3_id));

        let rows2 = ctrl.build_move_rows();
        assert_eq!(rows2[1].white_san.as_str(), "Nf3", "Nf3 est redevenu la ligne active");
        assert_eq!(rows2[1].black_san.as_str(), "Nc6", "sa suite déjà enregistrée (Nc6) est restaurée");
        assert_eq!(ctrl.viewed_ply_slint(), -1, "retour à la pointe de la ligne promue");
    }

    #[test]
    fn test_promote_variation_to_mainline_unknown_node_id_returns_false() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        assert!(!ctrl.promote_variation_to_mainline(9999));
    }

    #[test]
    fn test_remove_variation_removes_variation_block() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0);
        ctrl.set_variation_mode_enabled(true);
        ctrl.on_click(1, 2); ctrl.on_click(3, 2); // 1...c5 → e5 demoted to a variation

        let rows = ctrl.build_move_rows();
        let e5_variation_id = rows[0].black_variation_node_id;
        assert!(e5_variation_id >= 0);

        assert!(ctrl.remove_variation(e5_variation_id as usize));

        let rows2 = ctrl.build_move_rows();
        assert_eq!(rows2[0].black_variations.as_str(), "", "la variante supprimée ne doit plus apparaître");
        assert_eq!(rows2[0].black_san.as_str(), "c5", "la ligne active n'est pas affectée");
    }

    #[test]
    fn test_remove_variation_refuses_active_line_node() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;

        assert!(!ctrl.remove_variation(node_id));
        assert_eq!(ctrl.build_move_rows()[0].white_san.as_str(), "e4", "aucune modification");
    }

    #[test]
    fn test_remove_variation_unknown_node_id_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.remove_variation(9999));
    }

    // ── Context menu: inline comment (PHASE 16, Step 6.3) ─────────────────────

    #[test]
    fn test_set_move_comment_sets_and_reflects_in_build_move_rows() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4

        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;
        assert!(ctrl.set_move_comment(node_id, "Meilleur premier coup selon la théorie"));

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_comment.as_str(), "Meilleur premier coup selon la théorie");
    }

    #[test]
    fn test_set_move_comment_empty_text_clears_existing_comment() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;

        assert!(ctrl.set_move_comment(node_id, "un commentaire"));
        assert!(ctrl.set_move_comment(node_id, ""));

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_comment.as_str(), "", "chaîne vide efface le commentaire");
    }

    #[test]
    fn test_set_move_comment_overwrites_previous_text() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        let node_id = ctrl.build_move_rows()[0].white_node_id as usize;

        assert!(ctrl.set_move_comment(node_id, "premier texte"));
        assert!(ctrl.set_move_comment(node_id, "texte corrigé"));

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_comment.as_str(), "texte corrigé");
    }

    #[test]
    fn test_set_move_comment_unknown_node_id_returns_false() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        assert!(!ctrl.set_move_comment(9999, "texte"));
    }

    #[test]
    fn test_build_move_rows_comment_targets_correct_color() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5

        let black_id = ctrl.build_move_rows()[0].black_node_id as usize;
        assert!(ctrl.set_move_comment(black_id, "commentaire noir"));

        let rows = ctrl.build_move_rows();
        assert_eq!(rows[0].white_comment.as_str(), "", "le commentaire ne doit pas déborder sur l'autre coup");
        assert_eq!(rows[0].black_comment.as_str(), "commentaire noir");
    }

    // ── current_node_id / flatten_move_tree (PHASE 16, Step 3) ───────────────

    #[test]
    fn test_current_node_id_none_when_no_moves() {
        assert!(GameController::new().current_node_id().is_none());
    }

    #[test]
    fn test_current_node_id_tracks_tip_when_not_viewing_history() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move("e2e4"));
        let after_e4 = ctrl.current_node_id().unwrap();

        assert!(ctrl.apply_uci_move("e7e5"));
        let after_e5 = ctrl.current_node_id().unwrap();

        assert_ne!(after_e4, after_e5);
    }

    #[test]
    fn test_current_node_id_follows_viewed_ply() {
        let mut ctrl = GameController::new();
        ctrl.apply_uci_move("e2e4");
        ctrl.apply_uci_move("e7e5");
        let tip = ctrl.current_node_id().unwrap();

        assert!(ctrl.go_to_ply(0));
        let at_ply0 = ctrl.current_node_id().unwrap();
        assert_ne!(at_ply0, tip);

        assert!(ctrl.go_to_ply(-1));
        assert_eq!(ctrl.current_node_id().unwrap(), tip);
    }

    #[test]
    fn test_flatten_move_tree_empty_at_start() {
        assert!(GameController::new().flatten_move_tree().is_empty());
    }

    #[test]
    fn test_flatten_move_tree_all_mainline_depth_zero() {
        let mut ctrl = GameController::new();
        ctrl.apply_uci_move("e2e4");
        ctrl.apply_uci_move("e7e5");
        ctrl.apply_uci_move("g1f3");

        let flat = ctrl.flatten_move_tree();
        assert_eq!(flat.len(), 3);
        assert!(flat.iter().all(|n| n.depth == 0 && n.is_mainline));
    }

    #[test]
    fn test_current_fen_initial() {
        let ctrl = GameController::new();
        let fen = ctrl.current_fen();
        assert!(fen.starts_with("rnbqkbnr/pppppppp"));
        assert!(fen.contains(" w "));
    }

    #[test]
    fn test_move_count_initial() {
        assert_eq!(GameController::new().move_count(), 0);
    }

    #[test]
    fn test_move_count_after_move() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4);
        assert_eq!(ctrl.move_count(), 1);
    }

    // ── apply_uci_move ────────────────────────────────────────────────────────

    #[test]
    fn test_apply_uci_move_e2e4() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move("e2e4"));
        assert_eq!(ctrl.move_count(), 1);
        assert!(!ctrl.is_white_turn());
    }

    #[test]
    fn test_last_move_san_none_when_empty() {
        let ctrl = GameController::new();
        assert_eq!(ctrl.last_move_san(), None);
    }

    #[test]
    fn test_last_move_san_after_move() {
        let mut ctrl = GameController::new();
        ctrl.apply_uci_move("e2e4");
        assert_eq!(ctrl.last_move_san().as_deref(), Some("e4"));
        ctrl.apply_uci_move("e7e5");
        assert_eq!(ctrl.last_move_san().as_deref(), Some("e5"));
    }

    #[test]
    fn test_last_move_uci_none_when_empty() {
        let ctrl = GameController::new();
        assert_eq!(ctrl.last_move_uci(), None);
    }

    #[test]
    fn test_last_move_uci_after_move() {
        let mut ctrl = GameController::new();
        ctrl.apply_uci_move("e2e4");
        assert_eq!(ctrl.last_move_uci().as_deref(), Some("e2e4"));
        ctrl.apply_uci_move("e7e5");
        assert_eq!(ctrl.last_move_uci().as_deref(), Some("e7e5"));
    }

    #[test]
    fn test_apply_uci_move_marks_from_book_false() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move("e2e4"));
        let rows = ctrl.build_move_rows();
        assert!(!rows[0].white_from_book);
    }

    #[test]
    fn test_apply_uci_move_from_book_marks_flag() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move_from_book("e2e4"));
        assert_eq!(ctrl.move_count(), 1);
        assert!(!ctrl.is_white_turn());
        let rows = ctrl.build_move_rows();
        assert!(rows[0].white_from_book);
        assert!(!rows[0].black_from_book);
    }

    #[test]
    fn test_apply_uci_move_from_book_second_move_independent() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move_from_book("e2e4"));
        assert!(ctrl.apply_uci_move("e7e5")); // normal move, not from the book
        let rows = ctrl.build_move_rows();
        assert!(rows[0].white_from_book);
        assert!(!rows[0].black_from_book);
    }

    #[test]
    fn test_apply_uci_move_from_book_illegal_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.apply_uci_move_from_book("e2e5")); // illegal pawn jump
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_apply_uci_move_two_moves() {
        let mut ctrl = GameController::new();
        assert!(ctrl.apply_uci_move("e2e4"));
        assert!(ctrl.apply_uci_move("e7e5"));
        assert_eq!(ctrl.move_count(), 2);
        assert!(ctrl.is_white_turn());
    }

    #[test]
    fn test_apply_uci_move_illegal_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.apply_uci_move("e2e6")); // illegal
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_apply_uci_move_too_short_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.apply_uci_move("e2e"));
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_apply_uci_move_none_returns_false() {
        // (position already checkmate/stalemate). Must never be treated as a move.
        let mut ctrl = GameController::new();
        assert!(!ctrl.apply_uci_move("(none)"));
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_apply_uci_move_empty_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.apply_uci_move(""));
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_apply_uci_move_promotion_queen() {
        // Black king on a8, white pawn on e7, white king on e1
        let fen = "k7/4P3/8/8/8/8/8/4K3 w - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection: None, targets: HashSet::new(), last_move: None,
            viewed_ply: None, pending_promotion: None, pending_promo_is_white: false,
            pending_promotion_ply: None, variation_editing: false, assist_mode: false,
        };
        assert!(ctrl.apply_uci_move("e7e8q"));
        assert_eq!(ctrl.move_count(), 1);
        let squares = ctrl.build_squares();
        let e8 = squares.iter().find(|s| s.row == 0 && s.col == 4).unwrap();
        assert_eq!(e8.piece_char.as_str(), "wQ", "dame blanche attendue en e8");
    }

    #[test]
    fn test_apply_uci_move_promotion_default_queen() {
        // Promotion without specifying the piece → Queen by default
        let fen = "k7/4P3/8/8/8/8/8/4K3 w - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection: None, targets: HashSet::new(), last_move: None,
            viewed_ply: None, pending_promotion: None, pending_promo_is_white: false,
            pending_promotion_ply: None, variation_editing: false, assist_mode: false,
        };
        // No 5th character → Queen by default
        assert!(ctrl.apply_uci_move("e7e8"));
        let squares = ctrl.build_squares();
        let e8 = squares.iter().find(|s| s.row == 0 && s.col == 4).unwrap();
        assert_eq!(e8.piece_char.as_str(), "wQ");
    }

    // ── reset ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_reset_clears_game() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        assert_eq!(ctrl.move_count(), 2);
        ctrl.reset();
        assert_eq!(ctrl.move_count(), 0);
        assert!(ctrl.is_white_turn());
        assert_eq!(ctrl.viewed_ply_slint(), -1);
        assert!(!ctrl.has_pending_promotion());
    }

    #[test]
    fn test_reset_exits_variation_editing() {
        // PHASE 26: a new game must never start in
        // variation-editing mode (a bug found while preparing Step 3).
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.enter_variation_editing();
        assert!(ctrl.is_variation_editing());

        ctrl.reset();
        assert!(!ctrl.is_variation_editing());
    }

    // ── load_from_fen ─────────────────────────────────────────────────────────

    #[test]
    fn test_load_from_fen_valid() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        assert_eq!(ctrl.move_count(), 1);
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert!(ctrl.load_from_fen(fen));
        // The history is cleared — the position is just loaded
        assert_eq!(ctrl.move_count(), 0);
        assert!(!ctrl.is_white_turn()); // Black moves in this FEN
    }

    #[test]
    fn test_load_from_fen_invalid_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.load_from_fen("ceci n'est pas un FEN"));
        // The original game is not modified
        assert_eq!(ctrl.move_count(), 0);
        assert!(ctrl.is_white_turn());
    }

    #[test]
    fn test_load_from_fen_resets_selection() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); // select e2
        assert!(ctrl.selection.is_some());
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        ctrl.load_from_fen(fen);
        assert!(ctrl.selection.is_none());
        assert!(ctrl.targets.is_empty());
    }

    #[test]
    fn test_load_from_pgn_exits_variation_editing() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.enter_variation_editing();
        assert!(ctrl.is_variation_editing());

        assert!(ctrl.load_from_pgn("1. e4 e5 *").is_ok());
        assert!(!ctrl.is_variation_editing());
    }

    #[test]
    fn test_load_from_fen_exits_variation_editing() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.go_to_ply(0);
        ctrl.enter_variation_editing();
        assert!(ctrl.is_variation_editing());

        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert!(ctrl.load_from_fen(fen));
        assert!(!ctrl.is_variation_editing());
    }

    #[test]
    fn test_load_from_fen_startpos() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(ctrl.load_from_fen(start));
        assert_eq!(ctrl.move_count(), 0);
        assert!(ctrl.is_white_turn());
    }

    // ── undo_last_move ────────────────────────────────────────────────────────

    #[test]
    fn test_undo_empty_history_returns_false() {
        let mut ctrl = GameController::new();
        assert!(!ctrl.undo_last_move());
        assert_eq!(ctrl.move_count(), 0);
    }

    #[test]
    fn test_undo_one_move() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        assert_eq!(ctrl.move_count(), 1);
        assert!(ctrl.undo_last_move());
        assert_eq!(ctrl.move_count(), 0);
        assert!(ctrl.is_white_turn());
    }

    #[test]
    fn test_undo_two_moves() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        assert_eq!(ctrl.move_count(), 2);
        assert!(ctrl.undo_last_move());
        assert_eq!(ctrl.move_count(), 1);
        assert!(!ctrl.is_white_turn()); // Black's turn again
        assert!(ctrl.undo_last_move());
        assert_eq!(ctrl.move_count(), 0);
        assert!(ctrl.is_white_turn());
    }

    #[test]
    fn test_undo_clears_selection_and_targets() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4 — Black's turn
        ctrl.on_click(1, 3); // select d7 (black pawn)
        assert!(ctrl.selection.is_some());
        ctrl.undo_last_move();
        assert!(ctrl.selection.is_none());
        assert!(ctrl.targets.is_empty());
    }

    #[test]
    fn test_undo_updates_last_move() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.undo_last_move(); // undo 1...e5
        // last_move must point to 1.e4
        assert!(ctrl.last_move.is_some());
    }

    #[test]
    fn test_undo_after_all_moves_clears_last_move() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.undo_last_move();
        assert!(ctrl.last_move.is_none());
    }

    #[test]
    fn test_undo_exits_viewed_ply_mode() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 4); ctrl.on_click(3, 4); // 1...e5
        ctrl.go_to_ply(0); // viewing mode
        assert_eq!(ctrl.viewed_ply_slint(), 0);
        ctrl.undo_last_move(); // must leave viewing AND undo
        assert_eq!(ctrl.viewed_ply_slint(), -1);
        assert_eq!(ctrl.move_count(), 1); // 1...e5 undone
    }

    // ── Deferred promotion ────────────────────────────────────────────────────

    /// Checks that no promotion is pending on a normal position.
    #[test]
    fn test_no_pending_promotion_initially() {
        assert!(!GameController::new().has_pending_promotion());
    }

    /// Checks promotion detection via a constructed position:
    /// white pawn on e7 able to promote on e8.
    ///
    /// FEN: `k7/4P3/8/8/8/8/8/4K3 w - - 0 1`
    /// (black king on a8, e8 free — pawn can advance and promote)
    /// Move: e7 → e8 (Slint row=1,col=4 → row=0,col=4)
    #[test]
    fn test_pending_promotion_detected() {
        // Position with a white pawn on e7
        let fen = "k7/4P3/8/8/8/8/8/4K3 w - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection:              None,
            targets:                HashSet::new(),
            last_move:              None,
            viewed_ply:             None,
            pending_promotion:      None,
            pending_promo_is_white: false,
            pending_promotion_ply:  None,
            variation_editing: false,
            assist_mode: false,
        };

        // Select the pawn on e7 (Slint: row=1, col=4)
        ctrl.on_click(1, 4);
        // Move to e8 (Slint: row=0, col=4)
        let changed = ctrl.on_click(0, 4);
        assert!(changed, "doit signaler un changement");
        assert!(ctrl.has_pending_promotion(), "promotion doit être en attente");
        assert!(ctrl.pending_promo_is_white(), "c'est le pion blanc");
        assert_eq!(ctrl.move_count(), 0, "coup pas encore joué");
    }

    /// Checks that `complete_promotion()` plays the move and clears the pending promotion.
    #[test]
    fn test_complete_promotion_queen() {
        let fen = "k7/4P3/8/8/8/8/8/4K3 w - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection:              None,
            targets:                HashSet::new(),
            last_move:              None,
            viewed_ply:             None,
            pending_promotion:      None,
            pending_promo_is_white: false,
            pending_promotion_ply:  None,
            variation_editing: false,
            assist_mode: false,
        };

        ctrl.on_click(1, 4); // e7
        ctrl.on_click(0, 4); // e8 → promotion pending
        assert!(ctrl.has_pending_promotion());

        let ok = ctrl.complete_promotion(1); // 1 = Queen
        assert!(ok, "la promotion doit réussir");
        assert!(!ctrl.has_pending_promotion(), "plus de promotion en attente");
        assert_eq!(ctrl.move_count(), 1, "coup joué");

        // Check that e8 does contain a white Queen
        let squares = ctrl.build_squares();
        let e8 = squares.iter().find(|s| s.row == 0 && s.col == 4).unwrap();
        assert_eq!(e8.piece_char.as_str(), "wQ", "dame blanche en e8");
    }

    // ── Captured pieces ───────────────────────────────────────────────────────

    #[test]
    fn test_piece_value_standard_scale() {
        assert_eq!(piece_value(PieceKind::Pawn), 1);
        assert_eq!(piece_value(PieceKind::Knight), 3);
        assert_eq!(piece_value(PieceKind::Bishop), 3);
        assert_eq!(piece_value(PieceKind::Rook), 5);
        assert_eq!(piece_value(PieceKind::Queen), 9);
        assert_eq!(piece_value(PieceKind::King), 0);
    }

    #[test]
    fn test_compact_captures_groups_duplicates() {
        let kinds = [PieceKind::Pawn, PieceKind::Pawn, PieceKind::Rook];
        let compacted = GameController::compact_captures(&kinds, Color::Black);
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0].piece_code.as_str(), "bP");
        assert_eq!(compacted[0].count, 2);
        assert_eq!(compacted[1].piece_code.as_str(), "bR");
        assert_eq!(compacted[1].count, 1);
    }

    #[test]
    fn test_captured_summary_empty_at_start() {
        let ctrl = GameController::new();
        let (white_trophies, black_trophies, diff) = ctrl.captured_summary();
        assert!(white_trophies.is_empty());
        assert!(black_trophies.is_empty());
        assert_eq!(diff, 0);
    }

    #[test]
    fn test_captured_summary_after_pawn_capture() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 3); ctrl.on_click(3, 3); // 1...d5
        ctrl.on_click(4, 4); ctrl.on_click(3, 3); // 2.exd5 (capture)

        let (white_trophies, black_trophies, diff) = ctrl.captured_summary();
        assert_eq!(white_trophies.len(), 1);
        assert_eq!(white_trophies[0].piece_code.as_str(), "bP");
        assert_eq!(white_trophies[0].count, 1);
        assert!(black_trophies.is_empty());
        assert_eq!(diff, 1);
    }

    #[test]
    fn test_captured_summary_en_passant() {
        // White pawn on e5, black pawn on d7 (ready to push to d5 to open
        // the en-passant capture).
        let fen = "4k3/3p4/8/4P3/8/8/8/4K3 b - - 0 1";
        let game = ChessGame::from_fen(fen).expect("FEN valide");
        let mut ctrl = GameController {
            game,
            selection: None, targets: HashSet::new(), last_move: None,
            viewed_ply: None, pending_promotion: None, pending_promo_is_white: false,
            pending_promotion_ply: None, variation_editing: false, assist_mode: false,
        };
        ctrl.on_click(1, 3); ctrl.on_click(3, 3); // 1...d5 (opens the en-passant capture)
        ctrl.on_click(3, 4); ctrl.on_click(2, 3); // 2.exd6 e.p.

        let (white_trophies, black_trophies, diff) = ctrl.captured_summary();
        assert_eq!(white_trophies.len(), 1, "le pion noir pris en passant doit apparaître");
        assert_eq!(white_trophies[0].piece_code.as_str(), "bP");
        assert!(black_trophies.is_empty());
        assert_eq!(diff, 1);
    }

    #[test]
    fn test_captured_summary_respects_viewed_ply() {
        let mut ctrl = GameController::new();
        ctrl.on_click(6, 4); ctrl.on_click(4, 4); // 1.e4
        ctrl.on_click(1, 3); ctrl.on_click(3, 3); // 1...d5
        ctrl.on_click(4, 4); ctrl.on_click(3, 3); // 2.exd5 (capture)

        // Go back to just before the capture (ply 1 = position after 1...d5)
        ctrl.go_to_ply(1);
        let (white_trophies, _black_trophies, diff) = ctrl.captured_summary();
        assert!(white_trophies.is_empty(), "pas encore de capture à ce ply");
        assert_eq!(diff, 0);
    }
}
