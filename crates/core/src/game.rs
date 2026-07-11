//! Full state of a chess game.
//!
//! [`GameState`] orchestrates position, history, notation, and rules.
//! It is the entry point for playing, undoing a move, or inspecting the state.

use crate::{
    history::{History, MoveRecord},
    notation::move_to_san,
    rules::{game_status, make_move, GameStatus, IllegalMoveError},
    types::{
        chess_move::Move,
        fen::FenError,
        game_state::GameResult,
        piece::{Color, PieceKind},
        position::Position,
    },
};

/// Full state of a game.
#[derive(Debug, Clone)]
pub struct GameState {
    /// Position the game started from (for resetting).
    initial_fen: String,
    /// Current position.
    position: Position,
    /// History of moves played.
    history: History,
    /// Result of the game.
    pub result: GameResult,
    /// Repetition keys of every position traversed since the start
    /// of the game (or since the custom starting position), in
    /// chronological order. `position_keys[0]` = initial position;
    /// `position_keys.len() == move_count() + 1` at all times.
    /// Used to detect the threefold repetition rule.
    position_keys: Vec<String>,
}

impl GameState {
    /// Creates a new game from the standard starting position.
    #[must_use]
    pub fn new() -> Self {
        let position = Position::starting();
        let position_keys = vec![position.repetition_key()];
        Self {
            initial_fen: position.to_fen(),
            position,
            history: History::new(),
            result: GameResult::Ongoing,
            position_keys,
        }
    }

    /// Creates a game from a custom FEN position.
    ///
    /// # Errors
    ///
    /// Returns an error if the FEN is syntactically invalid, or if the
    /// position does not contain exactly one king per color
    /// ([`FenError::InvalidKingCount`]) — a playable game cannot
    /// start without this guarantee (`is_in_check`/mate/stalemate depend on it).
    /// This check is done here rather than in `Position::from_fen`
    /// (a low-level type reused by many internal tests with partial
    /// positions): this is the real boundary for any FEN
    /// provided by the user (paste FEN, wizard, position editor).
    pub fn from_fen(fen: &str) -> Result<Self, FenError> {
        let position = Position::from_fen(fen)?;

        let white_kings = position.board.piece_count(Color::White, PieceKind::King);
        let black_kings = position.board.piece_count(Color::Black, PieceKind::King);
        if white_kings != 1 || black_kings != 1 {
            return Err(FenError::InvalidKingCount { white: white_kings, black: black_kings });
        }

        let position_keys = vec![position.repetition_key()];
        Ok(Self {
            initial_fen: fen.to_owned(),
            position,
            history: History::new(),
            result: GameResult::Ongoing,
            position_keys,
        })
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Current position.
    #[must_use]
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Move history.
    #[must_use]
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Move history (mutable access) — PHASE 16, Step 6.1.
    ///
    /// Gives access to [`History::tree_mut`] for context-menu
    /// actions (NAG, comment, promotion, deletion) that only modify
    /// the tree, never `path`/the current position directly.
    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// Number of moves played.
    #[must_use]
    pub fn move_count(&self) -> usize {
        self.history.len()
    }

    /// Is the game over?
    #[must_use]
    pub fn is_over(&self) -> bool {
        self.result != GameResult::Ongoing
    }

    /// FEN of the initial position.
    #[must_use]
    pub fn initial_fen(&self) -> &str {
        &self.initial_fen
    }

    /// Marks the last move played as coming from a Polyglot opening
    /// book (PHASE 15) rather than from an engine computation or a human move.
    ///
    /// Does nothing if the history is empty. Only affects the display
    /// metadata of the move (📖 icon in the move list) — the move
    /// itself has already been played normally via [`Self::play`].
    pub fn mark_last_move_as_book(&mut self) {
        if let Some(rec) = self.history.last_mut() {
            rec.from_book = true;
        }
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    /// Plays move `m` in the current position.
    ///
    /// Records the move in the history with its SAN notation, updates
    /// the position, and computes the new game status.
    ///
    /// # Errors
    ///
    /// Returns [`IllegalMoveError`] if the move is invalid.
    pub fn play(&mut self, m: Move) -> Result<GameStatus, IllegalMoveError> {
        let fen_before = self.position.to_fen();
        let san        = move_to_san(&self.position, m);
        let new_pos    = make_move(&self.position, m)?;

        self.history.push(MoveRecord { mv: m, san, fen_before, from_book: false });
        self.position = new_pos;

        // Threefold repetition: has the position we just reached
        // already been seen at least twice before (so 3 times in total)?
        let key = self.position.repetition_key();
        self.position_keys.push(key.clone());
        let repetition_count = self.position_keys.iter().filter(|k| **k == key).count();

        let mut status = game_status(&self.position);
        if status == GameStatus::Ongoing && repetition_count >= 3 {
            status = GameStatus::DrawByRepetition;
        }
        self.update_result(status);

        Ok(status)
    }

    /// Undoes the last move played.
    ///
    /// Returns `true` if a move was undone, `false` if the history is empty.
    pub fn undo(&mut self) -> bool {
        if let Some(record) = self.history.pop() {
            match Position::from_fen(&record.fen_before) {
                Ok(pos) => {
                    self.position = pos;
                    self.result = GameResult::Ongoing;
                    // Remove the repetition key pushed by the undone move
                    // (keeps `position_keys.len() == move_count() + 1`).
                    self.position_keys.pop();
                    true
                }
                Err(e) => {
                    // The saved FEN is corrupted: put the move back in the history
                    // and report the failure (should never happen).
                    eprintln!("[game::undo] FEN interne invalide, undo annulé : {e}");
                    self.history.push(record);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Plays move `m` as a **variation** from the position reached after
    /// the move at index `ply` of the active line (PHASE 16, Step 5), rather
    /// than in the current position (see [`Self::play`]).
    ///
    /// PHASE 16 decision #1: playing a move from a position in the
    /// history never truncates the existing line beyond `ply` —
    /// it is kept in the tree (see [`History::branch_at`]) as a
    /// non-active continuation, only replaced as the displayed line
    /// by the new variation. If `ply` is the last index of the
    /// active line (nothing to keep beyond it), the result is
    /// identical to a normal [`Self::play`].
    ///
    /// # Errors
    ///
    /// Returns [`IllegalMoveError`] if `ply` is out of bounds or if `m` is
    /// not legal in the position reached after that move.
    pub fn play_variation(&mut self, ply: usize, m: Move) -> Result<GameStatus, IllegalMoveError> {
        let base_pos = self
            .position_at(ply + 1)
            .ok_or(IllegalMoveError::MoveNotLegal)?;

        let fen_before = base_pos.to_fen();
        let san        = move_to_san(&base_pos, m);
        let new_pos    = make_move(&base_pos, m)?;

        self.history
            .branch_at(ply, MoveRecord { mv: m, san, fen_before, from_book: false })
            .ok_or(IllegalMoveError::MoveNotLegal)?;
        self.position = new_pos;

        // The active line may have changed (a longer continuation may have
        // been set aside in favor of the variation): the repetition keys must
        // be recomputed entirely from the new line rather than
        // reusing `position_keys`, which reflected the old line.
        self.recompute_position_keys();

        let key = self.position.repetition_key();
        let repetition_count = self.position_keys.iter().filter(|k| **k == key).count();

        let mut status = game_status(&self.position);
        if status == GameStatus::Ongoing && repetition_count >= 3 {
            status = GameStatus::DrawByRepetition;
        }
        self.update_result(status);

        Ok(status)
    }

    /// Resynchronizes `position`, `position_keys`, and `result` with the tip
    /// of the active line of [`Self::history`], after a structural
    /// mutation performed directly via [`Self::history_mut`] (e.g.
    /// [`History::promote_to_mainline`]) that changed `path` — hence the
    /// active line — without going through [`Self::play`]/[`Self::play_variation`],
    /// which keep these fields up to date themselves.
    ///
    /// PHASE 16 bug fixed on 04/07/2026 (found by the Step 8 integration
    /// test): before this method was added, `promote_to_mainline`
    /// correctly realigned `path` (correct tree structure and `move_count()`),
    /// but `position`/`current_fen()` remained frozen on
    /// the old active line — silently wrong for the legality of
    /// subsequent moves, engine analysis, and FEN export.
    ///
    /// # Panics
    /// Does not panic in practice: `position_at(self.history.len())` replays
    /// the active line from `initial_fen`, which is always replayable by
    /// construction (every move in `history` was validated as legal at the
    /// time it was played).
    pub fn sync_position_with_history(&mut self) {
        self.position = self
            .position_at(self.history.len())
            .expect("la ligne active de l'historique est toujours rejouable depuis initial_fen");
        self.recompute_position_keys();

        let key = self.position.repetition_key();
        let repetition_count = self.position_keys.iter().filter(|k| **k == key).count();
        let mut status = game_status(&self.position);
        if status == GameStatus::Ongoing && repetition_count >= 3 {
            status = GameStatus::DrawByRepetition;
        }
        self.update_result(status);
    }

    /// Fully recomputes `position_keys` by replaying the active line
    /// (`history.moves()`) from `initial_fen` — used by
    /// [`Self::play_variation`] after an active-line change, where the
    /// keys accumulated so far no longer correspond to the new line.
    fn recompute_position_keys(&mut self) {
        let mut pos = Position::from_fen(&self.initial_fen)
            .expect("initial_fen toujours valide (vérifié à la création de GameState)");
        let mut keys = Vec::with_capacity(self.history.len() + 1);
        keys.push(pos.repetition_key());
        for mv in self.history.moves() {
            pos = crate::movegen::apply_move(&pos, mv)
                .expect("coup de l'historique toujours légal (déjà validé au moment où il a été joué)");
            keys.push(pos.repetition_key());
        }
        self.position_keys = keys;
    }

    /// Returns the position at index `index` of the history.
    ///
    /// - `0` = initial position
    /// - `n` = position after the n-th move
    ///
    /// Returns `None` if the index is out of bounds.
    ///
    /// Applies the recorded moves directly (`movegen::apply_move`)
    /// without revalidating their legality: they were already validated at the
    /// time they were played via [`Self::play`]. Avoids regenerating the list of
    /// legal moves for each intermediate position, which becomes
    /// noticeable during continuous navigation (slider) over a long
    /// game (perf audit 02/07/2026, point 4).
    ///
    /// Uses [`History::moves`] (PHASE 16, Step 2) rather than
    /// `History::records()`: since `History` is backed by a tree,
    /// `records()` clones the entire history on every call, which would
    /// reintroduce the cost that the perf audit aimed to eliminate on this
    /// precisely sensitive path; `moves()` walks the tree without copying.
    #[must_use]
    pub fn position_at(&self, index: usize) -> Option<Position> {
        if index == 0 {
            return Position::from_fen(&self.initial_fen).ok();
        }
        self.history
            .get(index - 1)
            .and_then(|_| {
                // Reconstruct the position by applying the moves from the beginning
                let mut pos = Position::from_fen(&self.initial_fen).ok()?;
                for mv in self.history.moves().take(index) {
                    pos = crate::movegen::apply_move(&pos, mv)?;
                }
                Some(pos)
            })
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn update_result(&mut self, status: GameStatus) {
        self.result = match status {
            GameStatus::Checkmate => match self.position.side_to_move {
                Color::White => GameResult::BlackWins,
                Color::Black => GameResult::WhiteWins,
            },
            GameStatus::Stalemate
            | GameStatus::DrawBy50MoveRule
            | GameStatus::DrawByInsufficientMaterial
            | GameStatus::DrawByRepetition => GameResult::Draw,
            GameStatus::Ongoing => GameResult::Ongoing,
        };
    }
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{chess_move::Move, piece::PieceKind, square::Square};

    fn sq(alg: &str) -> Square {
        Square::from_algebraic(alg).expect("case invalide")
    }

    fn mv(from: &str, to: &str) -> Move {
        Move::normal(sq(from), sq(to))
    }

    #[test]
    fn test_new_game() {
        let g = GameState::new();
        assert_eq!(g.move_count(), 0);
        assert!(!g.is_over());
        assert_eq!(g.result, GameResult::Ongoing);
    }

    #[test]
    fn test_history_mut_allows_setting_nag() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        let id = g.history().last_node_id().unwrap();

        g.history_mut().tree_mut().node_mut(id).unwrap().nag = Some(crate::game_tree::Nag::Good);

        assert_eq!(g.history().tree().node(id).unwrap().nag, Some(crate::game_tree::Nag::Good));
    }

    #[test]
    fn test_play_e4() {
        let mut g = GameState::new();
        let status = g.play(mv("e2", "e4")).unwrap();
        assert_eq!(status, GameStatus::Ongoing);
        assert_eq!(g.move_count(), 1);
        assert!(g.position().board.piece_at(sq("e4")).is_some());
        assert!(g.position().board.piece_at(sq("e2")).is_none());
    }

    #[test]
    fn test_mark_last_move_as_book() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        assert!(!g.history().last().unwrap().from_book);

        g.mark_last_move_as_book();
        assert!(g.history().last().unwrap().from_book);

        // A second normal move must not inherit the flag.
        g.play(mv("e7", "e5")).unwrap();
        assert!(!g.history().last().unwrap().from_book);
        // The first move stays marked.
        assert!(g.history().get(0).unwrap().from_book);
    }

    #[test]
    fn test_mark_last_move_as_book_empty_history_no_panic() {
        let mut g = GameState::new();
        g.mark_last_move_as_book(); // must not panic
        assert!(g.history().last().is_none());
    }

    #[test]
    fn test_play_san_recorded() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        assert_eq!(g.history().last().unwrap().san, "e4");
    }

    #[test]
    fn test_play_invalid_move() {
        let mut g = GameState::new();
        let result = g.play(mv("e2", "e5"));
        assert!(result.is_err());
    }

    #[test]
    fn test_undo() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        let undone = g.undo();
        assert!(undone);
        assert_eq!(g.move_count(), 0);
        assert!(g.position().board.piece_at(sq("e2")).is_some());
        assert!(g.position().board.piece_at(sq("e4")).is_none());
    }

    #[test]
    fn test_undo_empty_history() {
        let mut g = GameState::new();
        assert!(!g.undo());
    }

    #[test]
    fn test_play_sequence_scholars_mate() {
        // Mate in 4 moves: 1.e4 e5 2.Bc4 Nc6 3.Qh5 Nf6?? 4.Qxf7#
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();
        g.play(mv("f1", "c4")).unwrap();
        g.play(mv("b8", "c6")).unwrap();
        g.play(mv("d1", "h5")).unwrap();
        g.play(mv("g8", "f6")).unwrap();
        let status = g.play(Move::normal(sq("h5"), sq("f7"))).unwrap();

        assert_eq!(status, GameStatus::Checkmate);
        assert_eq!(g.result, GameResult::WhiteWins);
        assert!(g.is_over());
    }

    #[test]
    fn test_undo_after_checkmate() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();
        g.play(mv("f1", "c4")).unwrap();
        g.play(mv("b8", "c6")).unwrap();
        g.play(mv("d1", "h5")).unwrap();
        g.play(mv("g8", "f6")).unwrap();
        g.play(Move::normal(sq("h5"), sq("f7"))).unwrap();

        assert!(g.is_over());
        g.undo();
        assert!(!g.is_over());
        assert_eq!(g.result, GameResult::Ongoing);
    }

    #[test]
    fn test_position_at_index() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();

        // Initial position (index 0) → e2 occupied
        let pos0 = g.position_at(0).unwrap();
        assert!(pos0.board.piece_at(sq("e2")).is_some());

        // Position after 1.e4 (index 1) → e4 occupied
        let pos1 = g.position_at(1).unwrap();
        assert!(pos1.board.piece_at(sq("e4")).is_some());
        assert!(pos1.board.piece_at(sq("e2")).is_none());
    }

    #[test]
    fn test_from_fen_rejects_missing_black_king() {
        let fen = "8/8/8/8/8/8/8/4K3 w - - 0 1";
        assert!(matches!(
            GameState::from_fen(fen),
            Err(crate::types::fen::FenError::InvalidKingCount { white: 1, black: 0 })
        ));
    }

    #[test]
    fn test_from_fen_rejects_two_white_kings() {
        let fen = "4k3/8/8/8/8/8/8/3KK3 w - - 0 1";
        assert!(matches!(
            GameState::from_fen(fen),
            Err(crate::types::fen::FenError::InvalidKingCount { white: 2, black: 1 })
        ));
    }

    #[test]
    fn test_from_fen_rejects_no_kings_at_all() {
        let fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        assert!(matches!(
            GameState::from_fen(fen),
            Err(crate::types::fen::FenError::InvalidKingCount { white: 0, black: 0 })
        ));
    }

    #[test]
    fn test_from_fen() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let g = GameState::from_fen(fen).unwrap();
        assert_eq!(g.move_count(), 0);
        assert!(g.position().board.piece_at(sq("e4")).is_some());
    }

    #[test]
    fn test_threefold_repetition_draw() {
        // Both knights shuffle back and forth: the starting position
        // returns exactly identical every 4 half-moves (occurrences
        // at plies 0, 4, and 8 → draw declared right after the 8th half-move).
        let mut g = GameState::new();
        let sequence = [
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
        ];
        let mut last_status = GameStatus::Ongoing;
        for (from, to) in sequence {
            last_status = g.play(mv(from, to)).unwrap();
        }
        assert_eq!(last_status, GameStatus::DrawByRepetition);
        assert_eq!(g.result, GameResult::Draw);
        assert!(g.is_over());
    }

    #[test]
    fn test_no_repetition_draw_before_third_occurrence() {
        // Only 2 occurrences of the starting position (0 and 4) → no draw.
        let mut g = GameState::new();
        let sequence = [("g1", "f3"), ("g8", "f6"), ("f3", "g1"), ("f6", "g8")];
        let mut last_status = GameStatus::Ongoing;
        for (from, to) in sequence {
            last_status = g.play(mv(from, to)).unwrap();
        }
        assert_eq!(last_status, GameStatus::Ongoing);
        assert!(!g.is_over());
    }

    #[test]
    fn test_undo_keeps_repetition_keys_in_sync() {
        // After an undo, replaying the same sequence must not trigger the
        // draw prematurely (the keys from the undone move must disappear).
        let mut g = GameState::new();
        g.play(mv("g1", "f3")).unwrap();
        g.play(mv("g8", "f6")).unwrap();
        assert!(g.undo()); // undoes Nf6
        assert!(g.undo()); // undoes Nf3
        // Starting position restored: replaying 3 full round trips
        // must behave exactly like test_threefold_repetition_draw.
        let sequence = [
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
        ];
        let mut last_status = GameStatus::Ongoing;
        for (from, to) in sequence {
            last_status = g.play(mv(from, to)).unwrap();
        }
        assert_eq!(last_status, GameStatus::DrawByRepetition);
    }

    #[test]
    fn test_position_at_after_castling() {
        // Verifies that position_at (direct application via movegen::apply_move,
        // without legality revalidation) faithfully reproduces castling.
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();
        g.play(mv("g1", "f3")).unwrap();
        g.play(mv("b8", "c6")).unwrap();
        g.play(mv("f1", "c4")).unwrap();
        g.play(mv("g8", "f6")).unwrap();
        g.play(Move::castle(sq("e1"), sq("g1"))).unwrap();

        let pos = g.position_at(7).unwrap();
        assert!(pos.board.piece_at(sq("g1")).is_some(), "roi doit être en g1 après le petit roque");
        assert!(pos.board.piece_at(sq("f1")).is_some(), "tour doit être en f1 après le petit roque");
        assert!(pos.board.piece_at(sq("e1")).is_none());
        assert!(pos.board.piece_at(sq("h1")).is_none());
    }

    #[test]
    fn test_position_at_after_en_passant() {
        let mut g = GameState::from_fen("4k3/8/8/8/3pP3/8/8/4K3 b - e3 0 1").unwrap();
        let status = g.play(Move::en_passant(sq("d4"), sq("e3"))).unwrap();
        assert_eq!(status, GameStatus::Ongoing);

        let pos = g.position_at(1).unwrap();
        assert!(pos.board.piece_at(sq("e3")).is_some(), "pion noir doit être en e3");
        assert!(pos.board.piece_at(sq("e4")).is_none(), "pion blanc capturé en passant doit disparaître");
        assert!(pos.board.piece_at(sq("d4")).is_none());
    }

    #[test]
    fn test_promotion_recorded_in_san() {
        let mut g = GameState::from_fen("8/4P3/8/8/8/8/8/4K2k w - - 0 1").unwrap();
        g.play(Move::promotion(sq("e7"), sq("e8"), PieceKind::Queen)).unwrap();
        assert_eq!(g.history().last().unwrap().san, "e8=Q");
    }

    // ── play_variation (PHASE 16, Step 5) ───────────────────────────────────

    #[test]
    fn test_play_variation_creates_branch_keeps_mainline_in_tree() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();
        g.play(mv("g1", "f3")).unwrap();
        let old_e5_id  = g.history().node_id_at(1).unwrap();
        let old_nf3_id = g.history().node_id_at(2).unwrap();

        // From ply 0 (after 1.e4), play 1...d5 instead of 1...e5.
        let status = g.play_variation(0, mv("d7", "d5")).unwrap();
        assert_eq!(status, GameStatus::Ongoing);

        assert_eq!(g.move_count(), 2);
        assert_eq!(g.history().last().unwrap().san, "d5");

        // 1...e5 and 2.Nf3 remain in the tree, not deleted.
        assert!(g.history().tree().node(old_e5_id).is_some());
        assert!(g.history().tree().node(old_nf3_id).is_some());
        assert_eq!(g.history().tree().len(), 4); // e4, e5, Nf3, d5
    }

    #[test]
    fn test_play_variation_updates_position_and_move_count() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();

        g.play_variation(0, mv("c7", "c5")).unwrap();

        assert_eq!(g.move_count(), 2);
        assert!(g.position().board.piece_at(sq("c5")).is_some());
        assert!(g.position().board.piece_at(sq("c7")).is_none());
        assert!(g.position().board.piece_at(sq("e4")).is_some());
    }

    #[test]
    fn test_play_variation_illegal_move_returns_err_and_no_mutation() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();

        let before_tree_len = g.history().tree().len();
        // e2 is empty after 1.e4: no piece to move from that square.
        let result = g.play_variation(0, mv("e2", "e4"));
        assert!(result.is_err());
        assert_eq!(g.move_count(), 2, "aucune mutation en cas de coup illégal");
        assert_eq!(g.history().tree().len(), before_tree_len);
    }

    #[test]
    fn test_play_variation_out_of_range_ply_returns_err() {
        let mut g = GameState::new();
        assert!(g.play_variation(0, mv("e2", "e4")).is_err());
        assert_eq!(g.move_count(), 0);
    }

    #[test]
    fn test_play_variation_at_tip_behaves_like_normal_play() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();

        // ply 0 is the tip: nothing to set aside, equivalent to a normal play().
        let status = g.play_variation(0, mv("e7", "e5")).unwrap();
        assert_eq!(status, GameStatus::Ongoing);
        assert_eq!(g.move_count(), 2);
        assert_eq!(g.history().last().unwrap().san, "e5");
    }

    #[test]
    fn test_play_variation_chained_moves_within_new_branch() {
        let mut g = GameState::new();
        g.play(mv("e2", "e4")).unwrap();
        g.play(mv("e7", "e5")).unwrap();

        g.play_variation(0, mv("c7", "c5")).unwrap(); // 1...c5 instead of 1...e5
        // A normal move must continue the new branch, not the old one.
        g.play(mv("g1", "f3")).unwrap();

        assert_eq!(g.move_count(), 3);
        let sans: Vec<String> = (0..3).map(|i| g.history().get(i).unwrap().san.clone()).collect();
        assert_eq!(sans, ["e4", "c5", "Nf3"]);
    }

    #[test]
    fn test_play_variation_resets_stale_draw_by_repetition_status() {
        let mut g = GameState::new();
        let sequence = [
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
            ("g1", "f3"), ("g8", "f6"),
            ("f3", "g1"), ("f6", "g8"),
        ];
        for (from, to) in sequence {
            g.play(mv(from, to)).unwrap();
        }
        assert_eq!(g.result, GameResult::Draw, "nulle par répétition attendue avant la branche");

        // Branching right after the very first move (1.Nf3) sets aside
        // the entire rest of the repetitive line: the status must
        // no longer be "draw" afterward, and the set-aside repetition keys must
        // not keep counting.
        let status = g.play_variation(0, mv("d7", "d5")).unwrap();
        assert_eq!(status, GameStatus::Ongoing);
        assert_eq!(g.result, GameResult::Ongoing);
        assert_eq!(g.move_count(), 2); // Nf3, d5
    }
}
