//! Session logic for Puzzles / Training mode (PHASE 14, Step 4).
//!
//! This module has no dependency on any interface (Slint) element: it only
//! orchestrates the chess position and move sequence of a
//! [`db::repository::puzzle_repo::PuzzleRow`]. The wiring to the actual
//! chessboard (display, animation, "Show solution" / "Next puzzle" /
//! "Quit" buttons) is done in `gui::main` (PHASE 14, Step 6, via
//! [`MoveOutcome`] and [`PuzzleSession::try_move_uci`]); the displayed
//! statistics and final validation are still to come (Steps 7 to 9).
//!
//! ## Reminder of the `Moves` column format
//!
//! Moves are in UCI notation, separated by spaces, and strictly alternate
//! starting from index 0:
//!
//! - **even** index (0, 2, 4…): **opponent's** move, played
//!   automatically (the first one brings the CSV's `FEN` position to the
//!   position the user must actually solve);
//! - **odd** index (1, 3, 5…): move that the **user must find**.
//!
//! The sequence always ends on a human move (even total number of
//! moves) — this is the move that solves the puzzle.
//!
//! ## Technical pitfall: `Move::from_uci` and castling / en passant
//!
//! [`chess_core::types::chess_move::Move::from_uci`] has no knowledge of the
//! board: castling or an en passant capture in UCI notation (e.g. `e1g1`,
//! `e5d6`) is therefore systematically decoded as `MoveKind::Normal`, while
//! [`chess_core::movegen::apply_move`] needs the **real** `MoveKind`
//! (`Castle`/`EnPassant`) to move the rook or remove the captured pawn —
//! otherwise the move would simply be rejected as illegal (`Move`
//! implements `PartialEq` on all its fields, `kind` included). This is the same
//! pitfall already documented for Polyglot books (PHASE 15).
//!
//! The solution, already used elsewhere in the project (see
//! `GameController::apply_uci_move_impl`), is to **never** directly compare
//! or apply a `Move` obtained via `from_uci`: it is first resolved
//! against the list of actually legal moves of the current position
//! (searched by `from`/`to`/`promotion`), which gives the move
//! with its real `kind`. See [`resolve_legal_move`] and
//! [`same_move_ignoring_kind`].

use chess_core::{
    movegen::generate_legal_moves,
    rules::{self, GameStatus},
    types::{chess_move::Move, fen::FenError, piece::Color, position::Position},
};

use db::repository::puzzle_repo::{AttemptResult, PuzzleRow};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error preventing the creation of a [`PuzzleSession`] from a
/// [`PuzzleRow`].
///
/// In theory, [`db::repository::puzzle_repo::import_csv`] has already validated
/// the FEN and the UCI syntax of each move at import time — these errors
/// should therefore not occur in practice. They remain handled
/// explicitly (rather than an `unwrap`/panic) out of caution: a row
/// could in theory be inserted into the database through another path, and the
/// project's principle is to never crash on external data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PuzzleSessionError {
    /// The FEN of the puzzle row could not be parsed.
    InvalidFen(FenError),
    /// One of the moves in the `Moves` column is not valid UCI notation.
    InvalidMoveSyntax,
    /// Fewer than two moves in the sequence: at minimum the opponent's
    /// setup move (index 0) and one human move (index 1) are required.
    TooFewMoves,
    /// The move at index `ply` (always 0 here: the first opponent move,
    /// the only automatic move played at construction) turned out to be illegal
    /// in the starting position — sequence corrupted from the start.
    IllegalSequenceMove { ply: usize },
}

impl std::fmt::Display for PuzzleSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFen(e) => write!(f, "FEN de puzzle invalide : {e}"),
            Self::InvalidMoveSyntax => write!(f, "Séquence de coups invalide (notation UCI)"),
            Self::TooFewMoves => write!(f, "Séquence de coups trop courte (minimum 2)"),
            Self::IllegalSequenceMove { ply } => {
                write!(f, "Coup illégal dans la séquence du puzzle (demi-coup {ply})")
            }
        }
    }
}

impl std::error::Error for PuzzleSessionError {}

/// Result of a human move attempt via [`PuzzleSession::try_move`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// Correct move; the opponent's forced reply was chained
    /// automatically (move carried here, kind resolved — ready to be
    /// applied as-is on a real `GameController`), and at least one
    /// human move remains to be found.
    CorrectContinue(Move),
    /// Correct move and it was the last one in the sequence: puzzle solved.
    Solved,
    /// Correct human move, but the next opponent reply recorded
    /// in the source file turned out to be illegal in the resulting
    /// position (corrupted data). The sequence stops cleanly here
    /// ([`PuzzleSession::is_broken`] becomes true) — to be treated as an
    /// abandonment by the caller (see [`PuzzleSession::outcome_for_stats`]).
    CorrectButSequenceBroken,
    /// Incorrect move: matches neither the expected move nor (on the last
    /// move) an alternative mate-in-one. Position **unchanged** —
    /// unlimited retries, consistent with the validated design decision.
    Incorrect,
    /// Illegal move in the current position, or puzzle already finished. Should
    /// not happen via a UI that only offers legal moves —
    /// not counted as an attempt for statistics.
    Illegal,
}

// ---------------------------------------------------------------------------
// Resolving a raw UCI move against actually legal moves
// ---------------------------------------------------------------------------

/// Compares two moves ignoring their `kind` — needed to compare a
/// move actually played (correct kind, resolved from the legal moves) to a
/// move expected as decoded raw by `Move::from_uci` (kind potentially
/// wrong for castling/en passant). See the module note.
fn same_move_ignoring_kind(a: Move, b: Move) -> bool {
    a.from == b.from && a.to == b.to && a.promotion == b.promotion
}

/// Finds, among the actually legal moves of `pos`, the one that
/// matches `candidate` by `from`/`to`/`promotion` (`candidate`'s `kind`
/// is ignored — see the module note). Returns the move with
/// its real `kind`, ready to be applied via [`rules::make_move`].
///
/// Returns `None` if `candidate` matches no legal move of `pos`
/// (corrupted sequence move, or diverging position).
fn resolve_legal_move(pos: &Position, candidate: Move) -> Option<Move> {
    generate_legal_moves(pos)
        .into_iter()
        .find(|m| same_move_ignoring_kind(*m, candidate))
}

// ---------------------------------------------------------------------------
// PuzzleSession
// ---------------------------------------------------------------------------

/// Session for solving a single puzzle.
///
/// Built from a [`PuzzleRow`] already drawn from the database (see
/// [`db::repository::puzzle_repo::random_puzzle`]); immediately plays the
/// opponent's setup move, leaving [`Self::position`] in
/// the state the user must solve.
#[derive(Debug)]
pub struct PuzzleSession {
    puzzle: PuzzleRow,
    position: Position,
    /// Raw moves of the `Moves` column, as decoded by
    /// `Move::from_uci` (kind potentially incorrect for castling/en
    /// passant — see module note). Even indices = opponent (auto),
    /// odd = human.
    solution: Vec<Move>,
    /// Index of the **next** move to play in `solution`.
    step: usize,
    /// Side that must solve the puzzle (fixed for the whole session: it's
    /// the side to move right after the opponent's setup move,
    /// and it becomes the side to move again after each forced opponent reply).
    hero_color: Color,
    /// Number of incorrect moves attempted during the session — used by
    /// [`Self::outcome_for_stats`] for the validated counting rule (an
    /// abandonment with no wrong move attempted is neutral, not a failure) and, since
    /// PHASE 14 Step 7, to enrich the result banner ("Solved with
    /// N error(s)").
    wrong_attempts_count: u32,
    solved: bool,
    revealed: bool,
    /// Sequence interrupted by a recorded illegal move (corrupted
    /// data) — see [`MoveOutcome::CorrectButSequenceBroken`].
    broken: bool,
}

impl PuzzleSession {
    /// Builds a new session from a puzzle row and immediately plays
    /// the opponent's setup move (index 0 of the
    /// sequence).
    ///
    /// # Errors
    ///
    /// See the variants of [`PuzzleSessionError`]. Should in practice
    /// never happen for a row passed through
    /// [`db::repository::puzzle_repo::import_csv`] (already validated at import).
    pub fn new(puzzle: &PuzzleRow) -> Result<Self, PuzzleSessionError> {
        let position = Position::from_fen(&puzzle.fen).map_err(PuzzleSessionError::InvalidFen)?;

        let solution: Vec<Move> = puzzle
            .moves
            .split_whitespace()
            .map(Move::from_uci)
            .collect::<Option<Vec<_>>>()
            .ok_or(PuzzleSessionError::InvalidMoveSyntax)?;

        if solution.len() < 2 {
            return Err(PuzzleSessionError::TooFewMoves);
        }

        let mut session = Self {
            puzzle: puzzle.clone(),
            position,
            solution,
            step: 0,
            hero_color: Color::White, // provisional value, set right below
            wrong_attempts_count: 0,
            solved: false,
            revealed: false,
            broken: false,
        };

        // Move 0: opponent reply that brings about the position the player must
        // actually solve.
        session
            .auto_play_next()
            .ok_or(PuzzleSessionError::IllegalSequenceMove { ply: 0 })?;

        session.hero_color = session.position.side_to_move;

        Ok(session)
    }

    // -- Accessors ------------------------------------------------------------

    /// Current board position (to display).
    #[must_use]
    pub fn position(&self) -> &Position {
        &self.position
    }

    /// Side that must solve the puzzle — for the automatic orientation of
    /// the board (validated design decision: always from this
    /// side's point of view).
    #[must_use]
    pub fn hero_color(&self) -> Color {
        self.hero_color
    }

    /// Original puzzle row (rating, themes, etc. for display).
    #[must_use]
    pub fn puzzle(&self) -> &PuzzleRow {
        &self.puzzle
    }

    /// `true` if the puzzle was solved by the user (via
    /// [`Self::try_move`] only — never via [`Self::reveal_solution`]).
    #[must_use]
    pub fn is_solved(&self) -> bool {
        self.solved
    }

    /// `true` if at least one incorrect move was attempted during this
    /// session (for the statistics counting rule).
    #[must_use]
    pub fn wrong_attempt_made(&self) -> bool {
        self.wrong_attempts_count > 0
    }

    /// Number of incorrect moves attempted during this session (PHASE 14,
    /// Step 7) — to enrich the result banner on `gui::main`'s side
    /// ("Puzzle solved (2 mistakes)", "Solution revealed (after 1 mistake)"…).
    #[must_use]
    pub fn wrong_attempts_count(&self) -> u32 {
        self.wrong_attempts_count
    }

    /// `true` if [`Self::reveal_solution`] was called on this session.
    #[must_use]
    pub fn is_revealed(&self) -> bool {
        self.revealed
    }

    /// `true` if the sequence was interrupted by a recorded
    /// illegal move (corrupted data) — see [`MoveOutcome::CorrectButSequenceBroken`].
    #[must_use]
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// `true` if the session is finished (solved, revealed to the end, or
    /// interrupted by a corrupted sequence) — no more human move
    /// is expected.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.step >= self.solution.len() || self.broken
    }

    /// Result to record in the statistics (`puzzle_progress`) according to
    /// the counting rule validated with the user (03/07/2026):
    ///
    /// - Solved → always [`AttemptResult::Solved`], regardless of the
    ///   number of wrong moves attempted along the way.
    /// - Not solved (abandoned, quit, or solution revealed) but at least one
    ///   wrong move attempted → [`AttemptResult::Failed`].
    /// - Not solved and **no** wrong move attempted → `None`: neutral
    ///   attempt, the caller must **not** call
    ///   [`db::repository::puzzle_repo::record_attempt`] at all.
    #[must_use]
    pub fn outcome_for_stats(&self) -> Option<AttemptResult> {
        if self.solved {
            Some(AttemptResult::Solved)
        } else if self.wrong_attempts_count > 0 {
            Some(AttemptResult::Failed)
        } else {
            None
        }
    }

    // -- Actions ----------------------------------------------------------------

    /// Attempts to play the human move `mv`.
    ///
    /// `mv` must be a move already resolved as legal in
    /// [`Self::position`] (e.g. from `generate_legal_moves`, as
    /// `GameController` already does for board clicks) — see
    /// [`MoveOutcome::Illegal`] otherwise.
    ///
    /// On the **last** move of the sequence, also accepts any legal move
    /// different from the expected move if it itself delivers a checkmate
    /// (nuance documented by Lichess: several moves can win a
    /// puzzle that ends on a mate-in-one).
    pub fn try_move(&mut self, mv: Move) -> MoveOutcome {
        if self.is_finished() {
            return MoveOutcome::Illegal;
        }
        if !rules::is_legal_move(&self.position, mv) {
            return MoveOutcome::Illegal;
        }

        let expected = self.solution[self.step];
        let is_last_human_move = self.step == self.solution.len() - 1;

        let Ok(new_pos) = rules::make_move(&self.position, mv) else {
            // Should not happen: `is_legal_move` just confirmed it.
            return MoveOutcome::Illegal;
        };

        let is_alternative_mate =
            is_last_human_move && rules::game_status(&new_pos) == GameStatus::Checkmate;
        let matched = same_move_ignoring_kind(mv, expected) || is_alternative_mate;

        if !matched {
            self.wrong_attempts_count += 1;
            return MoveOutcome::Incorrect;
        }

        self.position = new_pos;
        self.step += 1;

        if self.step >= self.solution.len() {
            self.solved = true;
            return MoveOutcome::Solved;
        }

        if let Some(opponent_reply) = self.resolve_and_apply_next() {
            MoveOutcome::CorrectContinue(opponent_reply)
        } else {
            self.broken = true;
            MoveOutcome::CorrectButSequenceBroken
        }
    }

    /// Convenience variant of [`Self::try_move`] for a caller that only
    /// handles UCI strings (e.g. retrieved via
    /// `GameController::last_move_uci`) rather than raw `Move`s.
    ///
    /// Resolves `uci` against the actually legal moves of [`Self::position`]
    /// before delegating to [`Self::try_move`] (same castling/en-passant pitfall
    /// as the one documented at the top of the module — see [`resolve_legal_move`]).
    /// Returns [`MoveOutcome::Illegal`] if `uci` is not
    /// valid notation or matches no current legal move.
    #[must_use]
    pub fn try_move_uci(&mut self, uci: &str) -> MoveOutcome {
        let Some(raw) = Move::from_uci(uci) else {
            return MoveOutcome::Illegal;
        };
        let Some(resolved) = resolve_legal_move(&self.position, raw) else {
            return MoveOutcome::Illegal;
        };
        self.try_move(resolved)
    }

    /// Automatically plays all remaining moves of the solution (including
    /// the opponent replies), for display/animation on
    /// the board by the caller. Returns the list of moves actually
    /// played, in order (moves already resolved, ready to be applied as
    /// is to a `GameController` for example).
    ///
    /// **Never** counts as a resolution by the player — see
    /// [`Self::outcome_for_stats`], which applies the validated
    /// counting rule (neutral abandonment or failure depending on whether a
    /// wrong move was attempted before, never "succeeded").
    ///
    /// No effect if the session is already finished (returns an empty list).
    pub fn reveal_solution(&mut self) -> Vec<Move> {
        self.revealed = true;
        let mut played = Vec::new();

        while !self.is_finished() {
            if let Some(mv) = self.resolve_and_apply_next() {
                played.push(mv);
            } else {
                self.broken = true;
                break;
            }
        }

        played
    }

    // -- Internal ---------------------------------------------------------------

    /// Resolves `self.solution[self.step]` against the legal moves of the
    /// current position, applies it, and advances `self.step`. Returns the
    /// resolved move (correct kind) on success.
    fn resolve_and_apply_next(&mut self) -> Option<Move> {
        let raw = *self.solution.get(self.step)?;
        let resolved = resolve_legal_move(&self.position, raw)?;
        let new_pos = rules::make_move(&self.position, resolved).ok()?;
        self.position = new_pos;
        self.step += 1;
        Some(resolved)
    }

    /// Like [`Self::resolve_and_apply_next`], without exposing the played move —
    /// used when only the effect on `self.position`/`self.step` matters
    /// (construction, chaining the opponent's reply in
    /// [`Self::try_move`]).
    fn auto_play_next(&mut self) -> Option<()> {
        self.resolve_and_apply_next().map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// All the positions below are either the standard starting position
// reached by perfectly well-known opening moves (1.e4 e5 2.Nf3 Nc6),
// or manually derived square by square with explicit verification of
// each attack/block in the comments — since no engine is
// available in this environment to verify a made-up position,
// any "exotic" position not fully traced by hand was avoided.

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 2-move puzzle from the standard starting position:
    /// index 0 (opponent) = 1.e4, index 1 (human) = 1...e5. Serves as the basis for
    /// most of the unit tests below (sequence structure,
    /// no tactical interest).
    fn simple_puzzle() -> PuzzleRow {
        PuzzleRow {
            id: 1,
            puzzle_id: "test001".into(),
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".into(),
            moves: "e2e4 e7e5".into(),
            rating: 1200,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: "test".into(),
            game_url: None,
            opening_tags: None,
        }
    }

    /// Four-move puzzle (two human moves) built on the standard opening
    /// 1.e4 e5 2.Nf3 Nc6, to test the full chaining:
    /// opponent / human / opponent / human.
    fn four_ply_puzzle() -> PuzzleRow {
        PuzzleRow {
            id: 2,
            puzzle_id: "test002".into(),
            fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".into(),
            // 1.e4 (opponent, index 0) e7e5 (human, index 1)
            // 2.Nf3 (opponent, index 2) b8c6 (human, index 3)
            moves: "e2e4 e7e5 g1f3 b8c6".into(),
            rating: 900,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: String::new(),
            game_url: None,
            opening_tags: None,
        }
    }

    #[test]
    fn test_new_plays_first_move_automatically() {
        let session = PuzzleSession::new(&simple_puzzle()).unwrap();
        // After 1.e4, it's Black's turn to play.
        assert_eq!(session.position().side_to_move, Color::Black);
        assert_eq!(session.hero_color(), Color::Black);
        assert!(!session.is_finished());
    }

    #[test]
    fn test_new_rejects_too_few_moves() {
        let mut row = simple_puzzle();
        row.moves = "e2e4".into();
        assert_eq!(
            PuzzleSession::new(&row).unwrap_err(),
            PuzzleSessionError::TooFewMoves
        );
    }

    #[test]
    fn test_new_rejects_invalid_fen() {
        let mut row = simple_puzzle();
        row.fen = "not-a-fen".into();
        assert!(matches!(
            PuzzleSession::new(&row),
            Err(PuzzleSessionError::InvalidFen(_))
        ));
    }

    #[test]
    fn test_new_rejects_invalid_move_syntax() {
        let mut row = simple_puzzle();
        row.moves = "e2e4 not-a-move".into();
        assert_eq!(
            PuzzleSession::new(&row).unwrap_err(),
            PuzzleSessionError::InvalidMoveSyntax
        );
    }

    #[test]
    fn test_new_rejects_illegal_first_move() {
        let mut row = simple_puzzle();
        // e2e5: a three-square jump, illegal for a pawn from its starting
        // square (maximum two squares).
        row.moves = "e2e5 e7e5".into();
        assert_eq!(
            PuzzleSession::new(&row).unwrap_err(),
            PuzzleSessionError::IllegalSequenceMove { ply: 0 }
        );
    }

    #[test]
    fn test_try_move_correct_solves_two_ply_puzzle() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        let legal = generate_legal_moves(session.position());
        let mv = legal
            .iter()
            .find(|m| m.to_uci() == "e7e5")
            .copied()
            .unwrap();

        let outcome = session.try_move(mv);
        assert_eq!(outcome, MoveOutcome::Solved);
        assert!(session.is_solved());
        assert!(session.is_finished());
        assert_eq!(session.outcome_for_stats(), Some(AttemptResult::Solved));
    }

    #[test]
    fn test_try_move_incorrect_leaves_position_unchanged() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        let before = session.position().to_fen();
        let legal = generate_legal_moves(session.position());
        // A legal black move but different from the expected solution (e7e5).
        let wrong = legal
            .iter()
            .find(|m| m.to_uci() == "b8c6")
            .copied()
            .unwrap();

        let outcome = session.try_move(wrong);
        assert_eq!(outcome, MoveOutcome::Incorrect);
        assert_eq!(session.position().to_fen(), before);
        assert!(session.wrong_attempt_made());
        assert!(!session.is_finished());
    }

    #[test]
    fn test_wrong_attempts_count_increments_across_retries() {
        // Unlimited retries (design decision already validated): two distinct
        // wrong moves before finding the solution must bring the
        // counter to 2, without preventing the final resolution or changing the
        // statistics result (always Solved, regardless of the number
        // of errors — see PHASE 14, Step 7).
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        assert_eq!(session.wrong_attempts_count(), 0);

        let legal = generate_legal_moves(session.position());
        let wrong1 = legal.iter().find(|m| m.to_uci() == "b8c6").copied().unwrap();
        assert_eq!(session.try_move(wrong1), MoveOutcome::Incorrect);
        assert_eq!(session.wrong_attempts_count(), 1);

        let wrong2 = legal.iter().find(|m| m.to_uci() == "g8f6").copied().unwrap();
        assert_eq!(session.try_move(wrong2), MoveOutcome::Incorrect);
        assert_eq!(session.wrong_attempts_count(), 2);

        let correct = legal.iter().find(|m| m.to_uci() == "e7e5").copied().unwrap();
        assert_eq!(session.try_move(correct), MoveOutcome::Solved);
        assert_eq!(session.wrong_attempts_count(), 2);
        assert_eq!(session.outcome_for_stats(), Some(AttemptResult::Solved));
    }

    #[test]
    fn test_try_move_illegal_move_rejected() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        // e2 is empty after 1.e4: no piece can move from there.
        let illegal = Move::from_uci("e2e4").unwrap();
        let outcome = session.try_move(illegal);
        assert_eq!(outcome, MoveOutcome::Illegal);
        assert!(!session.wrong_attempt_made());
    }

    #[test]
    fn test_try_move_after_finished_returns_illegal() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        let legal = generate_legal_moves(session.position());
        let mv = legal.iter().find(|m| m.to_uci() == "e7e5").copied().unwrap();
        assert_eq!(session.try_move(mv), MoveOutcome::Solved);

        // Playing anything again after resolution must have no effect
        // (guarded by `is_finished()`, short-circuits before any
        // legality check).
        let any = Move::from_uci("g1f3").unwrap();
        assert_eq!(session.try_move(any), MoveOutcome::Illegal);
    }

    #[test]
    fn test_four_ply_puzzle_chains_opponent_reply_then_solves() {
        let mut session = PuzzleSession::new(&four_ply_puzzle()).unwrap();
        // After 1.e4, it's Black to move, who must play e7e5.
        assert_eq!(session.position().side_to_move, Color::Black);

        let legal = generate_legal_moves(session.position());
        let mv1 = legal.iter().find(|m| m.to_uci() == "e7e5").copied().unwrap();
        let outcome1 = session.try_move(mv1);
        // The opponent reply (2.Nf3, g1f3) must have been chained
        // automatically and carried in the variant: Black to move
        // again, with no intervention, and the returned move is indeed g1f3.
        match outcome1 {
            MoveOutcome::CorrectContinue(opponent_reply) => {
                assert_eq!(opponent_reply.to_uci(), "g1f3");
            }
            other => panic!("attendu CorrectContinue, obtenu {other:?}"),
        }
        assert_eq!(session.position().side_to_move, Color::Black);
        assert!(!session.is_finished());

        let legal2 = generate_legal_moves(session.position());
        let mv2 = legal2.iter().find(|m| m.to_uci() == "b8c6").copied().unwrap();
        let outcome2 = session.try_move(mv2);
        assert_eq!(outcome2, MoveOutcome::Solved);
        assert!(session.is_solved());
    }

    #[test]
    fn test_reveal_solution_does_not_count_as_solved() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        let played = session.reveal_solution();

        assert!(session.is_revealed());
        assert!(session.is_finished());
        assert!(!session.is_solved());
        assert_eq!(played.len(), 1); // a single human move remaining (e7e5)
        // No wrong move attempted before the reveal → neutral, not a failure.
        assert_eq!(session.outcome_for_stats(), None);
    }

    #[test]
    fn test_reveal_solution_after_wrong_attempt_counts_as_failed() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        let legal = generate_legal_moves(session.position());
        let wrong = legal
            .iter()
            .find(|m| m.to_uci() == "b8c6")
            .copied()
            .unwrap();
        assert_eq!(session.try_move(wrong), MoveOutcome::Incorrect);

        session.reveal_solution();

        assert!(!session.is_solved());
        assert_eq!(session.outcome_for_stats(), Some(AttemptResult::Failed));
    }

    #[test]
    fn test_outcome_for_stats_neutral_when_untouched() {
        let session = PuzzleSession::new(&simple_puzzle()).unwrap();
        // No attempt, no reveal: neutral by construction.
        assert_eq!(session.outcome_for_stats(), None);
    }

    #[test]
    fn test_castling_move_in_sequence_is_correctly_applied() {
        // Italian Game position after 1.e4 e5 2.Nf3 Nc6 3.Bc4 Bc5
        // (well-known reference FEN), White to move. The setup
        // move (index 0) is 4.O-O (e1g1): checks that the
        // castling is actually executed (king on g1 AND rook moved to f1),
        // and not just the king teleported without the rook — which would happen
        // if `Move::from_uci("e1g1")` (kind=Normal) were applied as-is
        // without going through `resolve_legal_move` (see module note).
        //
        // Manual verification of the legality of 4.O-O in this position:
        // squares f1/g1 empty, king and rook h1 have not moved (KQkq
        // rights intact), king not in check, and the diagonal of the black bishop
        // c5→g1 is blocked by the white pawn on f2 (so g1 is not attacked).
        let row = PuzzleRow {
            id: 3,
            puzzle_id: "test003".into(),
            fen: "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4".into(),
            moves: "e1g1 c5f2".into(),
            rating: 1000,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: String::new(),
            game_url: None,
            opening_tags: None,
        };

        let session = PuzzleSession::new(&row).unwrap();
        let placement = session.position().to_fen();
        let placement = placement.split(' ').next().unwrap();
        assert_eq!(
            placement,
            "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQ1RK1"
        );
        assert_eq!(session.position().side_to_move, Color::Black);
    }

    #[test]
    fn test_alternative_mate_accepted_when_different_from_expected() {
        // Position built and manually verified (no engine
        // available in this environment):
        //   White: Ra1, Pawn h2, White to move.
        //   Black: Rc2 (king), Qb8 (queen).
        // FEN: "1q6/8/8/8/8/8/2k4P/K7 w - - 0 1"
        //
        // Setup move (index 0): h2h3, a neutral white move that doesn't
        // change anything about the geometry of the mate (pawn far from the action).
        //
        // Once Black is to move, the queen on b8 can go down the
        // b-file to b1 OR b2, and both are mate:
        //   - Qb1+: queen adjacent to the a1 king (check on rank 1),
        //     defended by the black king c2 (adjacent diagonally) → the white
        //     king cannot capture; a2 and b2 are both attacked
        //     by the queen on b1 → no escape square → mate.
        //   - Qb2+: queen adjacent to the a1 king (diagonal check),
        //     defended by the black king c2 (adjacent horizontally) → the
        //     white king cannot capture; a2 and b1 are both
        //     attacked by the queen on b2 → no escape square → mate.
        // The move *recorded* in the sequence is Qb2 (b8b2); this test
        // checks that a player who finds the other mate (Qb1, b8b1) is still
        // recognized as having solved the puzzle.
        let row = PuzzleRow {
            id: 4,
            puzzle_id: "test004".into(),
            fen: "1q6/8/8/8/8/8/2k4P/K7 w - - 0 1".into(),
            moves: "h2h3 b8b2".into(),
            rating: 1000,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: String::new(),
            game_url: None,
            opening_tags: None,
        };

        let mut session = PuzzleSession::new(&row).unwrap();
        assert_eq!(session.position().side_to_move, Color::Black);

        let legal = generate_legal_moves(session.position());
        let alt_mv = legal
            .iter()
            .find(|m| m.to_uci() == "b8b1")
            .copied()
            .expect("Db1 doit être un coup légal dans cette position");

        assert_eq!(session.try_move(alt_mv), MoveOutcome::Solved);
        assert!(session.is_solved());
        assert_eq!(session.outcome_for_stats(), Some(AttemptResult::Solved));
    }

    #[test]
    fn test_non_mating_wrong_move_on_last_ply_still_incorrect() {
        // Same position as the previous test, but here a legal move is played
        // that doesn't mate at all (the black king retreats, for example): must
        // remain "Incorrect", the tolerance only applying to moves that
        // actually deliver a mate.
        let row = PuzzleRow {
            id: 5,
            puzzle_id: "test005".into(),
            fen: "1q6/8/8/8/8/8/2k4P/K7 w - - 0 1".into(),
            moves: "h2h3 b8b2".into(),
            rating: 1000,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: String::new(),
            game_url: None,
            opening_tags: None,
        };

        let mut session = PuzzleSession::new(&row).unwrap();
        let legal = generate_legal_moves(session.position());
        // Kc2-d3: legal black king move that has nothing to do with the mate.
        let non_mating = legal
            .iter()
            .find(|m| m.to_uci() == "c2d3")
            .copied()
            .expect("Rd3 doit être un coup légal dans cette position");

        assert_eq!(session.try_move(non_mating), MoveOutcome::Incorrect);
        assert!(session.wrong_attempt_made());
        assert!(!session.is_finished());
    }

    #[test]
    fn test_try_move_uci_solves_two_ply_puzzle() {
        // Same scenario as `test_try_move_correct_solves_two_ply_puzzle`,
        // but via the convenience variant `try_move_uci` (direct UCI string,
        // without going through `generate_legal_moves` on the caller's side).
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        assert_eq!(session.try_move_uci("e7e5"), MoveOutcome::Solved);
        assert!(session.is_solved());
    }

    #[test]
    fn test_try_move_uci_chains_opponent_reply() {
        let mut session = PuzzleSession::new(&four_ply_puzzle()).unwrap();
        match session.try_move_uci("e7e5") {
            MoveOutcome::CorrectContinue(opponent_reply) => {
                assert_eq!(opponent_reply.to_uci(), "g1f3");
            }
            other => panic!("attendu CorrectContinue, obtenu {other:?}"),
        }
        assert_eq!(session.try_move_uci("b8c6"), MoveOutcome::Solved);
    }

    #[test]
    fn test_try_move_uci_rejects_garbage_syntax() {
        let mut session = PuzzleSession::new(&simple_puzzle()).unwrap();
        assert_eq!(session.try_move_uci("not-a-move"), MoveOutcome::Illegal);
        assert!(!session.wrong_attempt_made());
    }

    #[test]
    fn test_try_move_uci_castling_resolved_correctly() {
        // Reuses the position from `test_castling_move_in_sequence_is_correctly_applied`
        // but validates the second move (c5f2, capture of the f2 pawn by the black bishop)
        // via `try_move_uci`, to cover the UCI resolution → full legal move
        // path (from/to/promotion) once castling has already been played as the
        // setup move.
        let row = PuzzleRow {
            id: 6,
            puzzle_id: "test006".into(),
            fen: "r1bqk1nr/pppp1ppp/2n5/2b1p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4".into(),
            moves: "e1g1 c5f2".into(),
            rating: 1000,
            rating_deviation: None,
            popularity: None,
            nb_plays: None,
            themes: String::new(),
            game_url: None,
            opening_tags: None,
        };
        let mut session = PuzzleSession::new(&row).unwrap();
        assert_eq!(session.try_move_uci("c5f2"), MoveOutcome::Solved);
    }
}
