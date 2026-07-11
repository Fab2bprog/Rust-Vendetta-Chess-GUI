//! Validation of chess rules and endgame detection.
//!
//! This module relies on [`crate::movegen`] for move generation
//! and adds higher-level rules logic on top:
//!
//! - Game status (ongoing, checkmate, stalemate, draws)
//! - Validation of a given move
//! - Applying a validated move

use crate::{
    movegen::{apply_move, generate_legal_moves, is_in_check},
    types::{
        board::Board,
        chess_move::Move,
        piece::{Color, PieceKind},
        position::Position,
        square::Square,
    },
};


// ---------------------------------------------------------------------------
// Game status
// ---------------------------------------------------------------------------

/// Current state of a chess game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    /// The game is in progress.
    Ongoing,
    /// The side to move is checkmated.
    Checkmate,
    /// The side to move has no legal move but is not in check.
    Stalemate,
    /// Draw by the 50-move rule (100 half-moves without a capture or pawn advance).
    DrawBy50MoveRule,
    /// Draw by insufficient material to mate.
    DrawByInsufficientMaterial,
    /// Draw by threefold repetition of the same position.
    DrawByRepetition,
}

impl GameStatus {
    /// Returns `true` if the game is over.
    #[must_use]
    #[inline]
    pub fn is_over(self) -> bool {
        !matches!(self, Self::Ongoing)
    }

    /// Returns `true` if the game is a draw.
    #[must_use]
    #[inline]
    pub fn is_draw(self) -> bool {
        matches!(
            self,
            Self::Stalemate
                | Self::DrawBy50MoveRule
                | Self::DrawByInsufficientMaterial
                | Self::DrawByRepetition
        )
    }
}

impl std::fmt::Display for GameStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Ongoing                   => "En cours",
            Self::Checkmate                 => "Échec et mat",
            Self::Stalemate                 => "Pat",
            Self::DrawBy50MoveRule          => "Nulle (règle des 50 coups)",
            Self::DrawByInsufficientMaterial => "Nulle (matériel insuffisant)",
            Self::DrawByRepetition          => "Nulle (répétition triple)",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// Insufficient material detection
// ---------------------------------------------------------------------------

/// Returns `true` if the board does not contain enough material
/// to force a mate (even with best play from both sides).
///
/// Recognized cases:
/// - King vs King
/// - King + Bishop vs King
/// - King + Knight vs King
/// - King + Bishop vs King + Bishop (both bishops on the same color square)
#[must_use]
pub fn has_insufficient_material(board: &Board) -> bool {
    // Non-king pieces of each side
    let white_minors: Vec<_> = board
        .pieces_of_color(Color::White)
        .filter(|(_, p)| !matches!(p.kind, PieceKind::King))
        .collect();
    let black_minors: Vec<_> = board
        .pieces_of_color(Color::Black)
        .filter(|(_, p)| !matches!(p.kind, PieceKind::King))
        .collect();

    match (white_minors.len(), black_minors.len()) {
        // King vs King
        (0, 0) => true,

        // King + minor piece vs King alone
        (1, 0) => matches!(white_minors[0].1.kind, PieceKind::Bishop | PieceKind::Knight),
        (0, 1) => matches!(black_minors[0].1.kind, PieceKind::Bishop | PieceKind::Knight),

        // King + Bishop vs King + Bishop
        (1, 1) => {
            let w = white_minors[0];
            let b = black_minors[0];
            if matches!(w.1.kind, PieceKind::Bishop) && matches!(b.1.kind, PieceKind::Bishop) {
                // Draw only if both bishops are on the same square color
                square_color(w.0) == square_color(b.0)
            } else {
                false
            }
        }

        _ => false,
    }
}

/// Square color: 0 = light square, 1 = dark square.
fn square_color(sq: Square) -> u8 {
    (sq.file() + sq.rank()) % 2
}

// ---------------------------------------------------------------------------
// Game status
// ---------------------------------------------------------------------------

/// Computes the current status of the game.
///
/// The check order matters:
/// 1. Insufficient material (immediate draw)
/// 2. 50-move rule
/// 3. Mate or stalemate (requires generating legal moves)
#[must_use]
pub fn game_status(pos: &Position) -> GameStatus {
    if has_insufficient_material(&pos.board) {
        return GameStatus::DrawByInsufficientMaterial;
    }

    if pos.halfmove_clock >= 100 {
        return GameStatus::DrawBy50MoveRule;
    }

    let legal_moves = generate_legal_moves(pos);

    if legal_moves.is_empty() {
        if is_in_check(pos) {
            GameStatus::Checkmate
        } else {
            GameStatus::Stalemate
        }
    } else {
        GameStatus::Ongoing
    }
}

// ---------------------------------------------------------------------------
// Illegal move error
// ---------------------------------------------------------------------------

/// Error returned by [`make_move`] if the move is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IllegalMoveError {
    /// No piece on the starting square.
    NoPieceAtSource,
    /// The piece on the starting square belongs to the opponent.
    WrongColorPiece,
    /// The move is not legal in this position.
    MoveNotLegal,
}

impl std::fmt::Display for IllegalMoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPieceAtSource  => write!(f, "Aucune pièce sur la case de départ"),
            Self::WrongColorPiece  => write!(f, "La pièce appartient à l'adversaire"),
            Self::MoveNotLegal     => write!(f, "Coup illégal dans cette position"),
        }
    }
}

impl std::error::Error for IllegalMoveError {}

// ---------------------------------------------------------------------------
// Move validation and application
// ---------------------------------------------------------------------------

/// Returns `true` if move `m` is legal in position `pos`.
#[must_use]
pub fn is_legal_move(pos: &Position, m: Move) -> bool {
    generate_legal_moves(pos).contains(&m)
}

/// Applies move `m` to position `pos` and returns the new position.
///
/// # Errors
///
/// Returns [`IllegalMoveError`] if:
/// - The starting square is empty
/// - The piece does not belong to the side to move
/// - The move is not in the list of legal moves
pub fn make_move(pos: &Position, m: Move) -> Result<Position, IllegalMoveError> {
    // Check the starting square
    let piece = pos
        .board
        .piece_at(m.from)
        .ok_or(IllegalMoveError::NoPieceAtSource)?;

    // Check the color
    if piece.color != pos.side_to_move {
        return Err(IllegalMoveError::WrongColorPiece);
    }

    // Check legality
    let legal = generate_legal_moves(pos);
    if !legal.contains(&m) {
        return Err(IllegalMoveError::MoveNotLegal);
    }

    // apply_move should never return None here (legal move validated above)
    apply_move(pos, m).ok_or(IllegalMoveError::MoveNotLegal)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{chess_move::Move, Position};
    // `Piece` is only used in these tests (clippy::items_after_statements,
    // post-audit fixes of 04/07/2026) — imported here rather than at the
    // module level to avoid triggering `unused_imports` in a normal build.
    use crate::types::piece::Piece;

    fn pos(fen: &str) -> Position {
        Position::from_fen(fen).expect("FEN invalide")
    }

    fn sq(alg: &str) -> Square {
        Square::from_algebraic(alg).expect("case invalide")
    }

    // --- GameStatus ---

    #[test]
    fn test_status_ongoing_start() {
        assert_eq!(game_status(&Position::starting()), GameStatus::Ongoing);
    }

    #[test]
    fn test_status_checkmate_fools_mate() {
        let p = pos("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3");
        assert_eq!(game_status(&p), GameStatus::Checkmate);
    }

    #[test]
    fn test_status_stalemate() {
        let p = pos("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
        assert_eq!(game_status(&p), GameStatus::Stalemate);
    }

    #[test]
    fn test_status_draw_50_move_rule() {
        // halfmove_clock = 100 → immediate draw (rook = sufficient material, not insufficient material)
        let p = pos("7k/8/8/8/8/8/8/R6K w - - 100 80");
        assert_eq!(game_status(&p), GameStatus::DrawBy50MoveRule);
    }

    #[test]
    fn test_status_not_draw_at_99_halfmoves() {
        let p = pos("8/8/8/8/8/8/8/4K2k w - - 99 80");
        // Two kings: insufficient material takes priority
        assert_eq!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    // --- Insufficient material ---

    #[test]
    fn test_insufficient_king_vs_king() {
        let p = pos("8/8/8/8/8/8/8/4K2k w - - 0 1");
        assert_eq!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    #[test]
    fn test_insufficient_king_bishop_vs_king() {
        let p = pos("8/8/8/8/8/8/8/4KB1k w - - 0 1");
        assert_eq!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    #[test]
    fn test_insufficient_king_knight_vs_king() {
        let p = pos("8/8/8/8/8/8/8/4KN1k w - - 0 1");
        assert_eq!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    #[test]
    fn test_sufficient_king_rook_vs_king() {
        let p = pos("8/8/8/8/8/8/8/4KR1k w - - 0 1");
        // Rook = sufficient material
        assert_ne!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    #[test]
    fn test_sufficient_king_queen_vs_king() {
        let p = pos("8/8/8/8/8/8/8/4KQ1k w - - 0 1");
        assert_ne!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    #[test]
    fn test_insufficient_bishop_vs_bishop_same_color() {
        // White bishop on c1 (light square: (2+0)%2=0), black bishop on d8 (light square: (3+7)%2=0)
        let p = pos("3b4/8/8/8/8/8/8/2B2K1k w - - 0 1");
        assert_eq!(game_status(&p), GameStatus::DrawByInsufficientMaterial);
    }

    // --- is_legal_move ---

    #[test]
    fn test_is_legal_move_valid() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e4"));
        assert!(is_legal_move(&p, m));
    }

    #[test]
    fn test_is_legal_move_invalid_no_piece() {
        let p = Position::starting();
        let m = Move::normal(sq("e4"), sq("e5")); // e4 empty in the starting position
        assert!(!is_legal_move(&p, m));
    }

    #[test]
    fn test_is_legal_move_leaves_king_in_check() {
        // White king e1, white rook e4 pinned by black rook e8
        let p = pos("4r3/8/8/8/4R3/8/8/4K3 w - - 0 1");
        // The rook cannot leave the e-file
        let illegal = Move::normal(sq("e4"), sq("d4"));
        assert!(!is_legal_move(&p, illegal));
    }

    // --- make_move ---

    #[test]
    fn test_make_move_valid() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e4"));
        let new_pos = make_move(&p, m).unwrap();
        // The pawn is now on e4
        assert!(new_pos.board.piece_at(sq("e4")).is_some());
        assert!(new_pos.board.piece_at(sq("e2")).is_none());
    }

    #[test]
    fn test_make_move_updates_side_to_move() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e4"));
        let new_pos = make_move(&p, m).unwrap();
        assert_eq!(new_pos.side_to_move, Color::Black);
    }

    #[test]
    fn test_make_move_updates_en_passant() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e4"));
        let new_pos = make_move(&p, m).unwrap();
        assert_eq!(new_pos.en_passant, Some(sq("e3")));
    }

    #[test]
    fn test_make_move_no_piece_at_source() {
        let p = Position::starting();
        let m = Move::normal(sq("e4"), sq("e5"));
        assert!(matches!(make_move(&p, m), Err(IllegalMoveError::NoPieceAtSource)));
    }

    #[test]
    fn test_make_move_wrong_color() {
        let p = Position::starting(); // white to move
        // Try to move a black piece
        let m = Move::normal(sq("e7"), sq("e5"));
        assert!(matches!(make_move(&p, m), Err(IllegalMoveError::WrongColorPiece)));
    }

    #[test]
    fn test_make_move_illegal_move() {
        let p = Position::starting();
        // The a1 rook cannot move (blocked)
        let m = Move::normal(sq("a1"), sq("a3"));
        assert!(matches!(make_move(&p, m), Err(IllegalMoveError::MoveNotLegal)));
    }

    #[test]
    fn test_make_move_castling() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let m = Move::castle(sq("e1"), sq("g1"));
        let new_pos = make_move(&p, m).unwrap();
        // King on g1, rook on f1
        assert_eq!(
            new_pos.board.piece_at(sq("g1")),
            Some(Piece::new(Color::White, PieceKind::King))
        );
        assert_eq!(
            new_pos.board.piece_at(sq("f1")),
            Some(Piece::new(Color::White, PieceKind::Rook))
        );
        assert!(new_pos.board.piece_at(sq("e1")).is_none());
        assert!(new_pos.board.piece_at(sq("h1")).is_none());
    }

    #[test]
    fn test_make_move_castling_removes_rights() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let m = Move::castle(sq("e1"), sq("g1"));
        let new_pos = make_move(&p, m).unwrap();
        assert!(!new_pos.castling.white_kingside);
        assert!(!new_pos.castling.white_queenside);
    }

    #[test]
    fn test_make_move_promotion() {
        let p = pos("8/4P3/8/8/8/8/8/4K3 w - - 0 1");
        let m = Move::promotion(sq("e7"), sq("e8"), PieceKind::Queen);
        let new_pos = make_move(&p, m).unwrap();
        assert_eq!(
            new_pos.board.piece_at(sq("e8")),
            Some(Piece::new(Color::White, PieceKind::Queen))
        );
    }

    // --- is_over / is_draw ---

    #[test]
    fn test_game_status_is_over() {
        assert!(!GameStatus::Ongoing.is_over());
        assert!(GameStatus::Checkmate.is_over());
        assert!(GameStatus::Stalemate.is_over());
        assert!(GameStatus::DrawBy50MoveRule.is_over());
        assert!(GameStatus::DrawByInsufficientMaterial.is_over());
        assert!(GameStatus::DrawByRepetition.is_over());
    }

    #[test]
    fn test_game_status_is_draw() {
        assert!(!GameStatus::Checkmate.is_draw());
        assert!(GameStatus::Stalemate.is_draw());
        assert!(GameStatus::DrawBy50MoveRule.is_draw());
        assert!(GameStatus::DrawByInsufficientMaterial.is_draw());
        assert!(GameStatus::DrawByRepetition.is_draw());
    }
}
