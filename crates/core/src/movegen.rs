//! Legal move generation and attack detection.
//!
//! # Algorithm
//!
//! 1. **Pseudo-legal generation**: moves valid according to each piece's
//!    movement rules, without accounting for checks.
//! 2. **Temporary application**: each move is applied to a copy of the
//!    position.
//! 3. **Legal filtering**: only moves where the side-to-move's king
//!    is not in check after the move are kept.

// The i8 → u8 casts after bounds checking (0 ≤ val < 8) are always safe.
// The u8 → i8 casts (file/rank, always 0..=7) can never "wrap"
// (clippy::cast_possible_wrap, post-audit fixes of 04/07/2026).
#![allow(clippy::cast_sign_loss, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

use crate::types::{
    board::Board,
    chess_move::{Move, MoveKind},
    piece::{Color, Piece, PieceKind},
    position::Position,
    square::Square,
};

// ---------------------------------------------------------------------------
// Movement constants (file_delta, rank_delta)
// ---------------------------------------------------------------------------

const KNIGHT_DELTAS: [(i8, i8); 8] = [
    (-2, -1), (-2, 1), (-1, -2), (-1, 2),
    (1, -2),  (1, 2),  (2, -1),  (2, 1),
];
const BISHOP_DELTAS: [(i8, i8); 4] = [(-1, -1), (-1, 1), (1, -1), (1, 1)];
const ROOK_DELTAS:   [(i8, i8); 4] = [(-1, 0),  (1, 0),  (0, -1), (0, 1)];
const QUEEN_DELTAS:  [(i8, i8); 8] = [
    (-1, -1), (-1, 0), (-1, 1),
    (0, -1),  (0, 1),
    (1, -1),  (1, 0),  (1, 1),
];
const KING_DELTAS: [(i8, i8); 8] = [
    (-1, -1), (-1, 0), (-1, 1),
    (0, -1),  (0, 1),
    (1, -1),  (1, 0),  (1, 1),
];

/// Promotion pieces in decreasing order of value.
const PROMOTION_PIECES: [PieceKind; 4] = [
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
];

// ---------------------------------------------------------------------------
// Utility: offset square
// ---------------------------------------------------------------------------

/// Returns the square `sq + (df, dr)`, or `None` if off the board.
#[inline]
fn offset_square(sq: Square, df: i8, dr: i8) -> Option<Square> {
    let f = sq.file() as i8 + df;
    let r = sq.rank() as i8 + dr;
    if (0..8).contains(&f) && (0..8).contains(&r) {
        Some(Square::new(f as u8, r as u8))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Attack detection
// ---------------------------------------------------------------------------

/// Returns `true` if square `sq` is attacked by at least one piece of `by_color`.
///
/// Used for:
/// - detecting checks (`is_in_check`)
/// - validating castling (forbidding passage through an attacked square)
/// - filtering king moves
#[must_use]
pub fn is_square_attacked(board: &Board, sq: Square, by_color: Color) -> bool {
    // --- Pawns ---
    // A white pawn attacks from one rank below (rank - 1).
    // A black pawn attacks from one rank above (rank + 1).
    let pawn_rank_dir: i8 = match by_color {
        Color::White => -1,
        Color::Black =>  1,
    };
    for df in [-1i8, 1i8] {
        if let Some(s) = offset_square(sq, df, pawn_rank_dir) {
            if board.piece_at(s) == Some(Piece::new(by_color, PieceKind::Pawn)) {
                return true;
            }
        }
    }

    // --- Knights ---
    for &(df, dr) in &KNIGHT_DELTAS {
        if let Some(s) = offset_square(sq, df, dr) {
            if board.piece_at(s) == Some(Piece::new(by_color, PieceKind::Knight)) {
                return true;
            }
        }
    }

    // --- Bishops / Queens (diagonals) ---
    for &(df, dr) in &BISHOP_DELTAS {
        let mut cur = sq;
        loop {
            match offset_square(cur, df, dr) {
                None => break,
                Some(next) => {
                    cur = next;
                    match board.piece_at(cur) {
                        None => {}
                        Some(p) if p.color == by_color
                            && matches!(p.kind, PieceKind::Bishop | PieceKind::Queen) =>
                        {
                            return true;
                        }
                        Some(_) => break, // piece blocking the ray
                    }
                }
            }
        }
    }

    // --- Rooks / Queens (orthogonals) ---
    for &(df, dr) in &ROOK_DELTAS {
        let mut cur = sq;
        loop {
            match offset_square(cur, df, dr) {
                None => break,
                Some(next) => {
                    cur = next;
                    match board.piece_at(cur) {
                        None => {}
                        Some(p) if p.color == by_color
                            && matches!(p.kind, PieceKind::Rook | PieceKind::Queen) =>
                        {
                            return true;
                        }
                        Some(_) => break,
                    }
                }
            }
        }
    }

    // --- King ---
    for &(df, dr) in &KING_DELTAS {
        if let Some(s) = offset_square(sq, df, dr) {
            if board.piece_at(s) == Some(Piece::new(by_color, PieceKind::King)) {
                return true;
            }
        }
    }

    false
}

/// Returns `true` if the side to move is in check.
#[must_use]
pub fn is_in_check(pos: &Position) -> bool {
    let color = pos.side_to_move;
    pos.board
        .find_king(color)
        .is_some_and(|king_sq| is_square_attacked(&pos.board, king_sq, color.opposite()))
}

// ---------------------------------------------------------------------------
// Applying a move (temporary copy for legality testing)
// ---------------------------------------------------------------------------

/// Applies `m` to a copy of `pos` and returns the new position.
///
/// Returns `None` if the position is inconsistent (empty source square,
/// missing promotion piece, missing rook for castling).
/// These cases should not occur with legally generated moves.
pub(crate) fn apply_move(pos: &Position, m: Move) -> Option<Position> {
    let mut new_pos = pos.clone();

    let piece = new_pos.board.piece_at(m.from)?;

    // Detect a capture BEFORE moving the pieces
    let is_capture = pos.board.piece_at(m.to).is_some()
        || m.kind == MoveKind::EnPassant;

    // Remove the piece from the starting square
    new_pos.board.set_piece(m.from, None);

    match m.kind {
        MoveKind::Normal => {
            new_pos.board.set_piece(m.to, Some(piece));
        }
        MoveKind::Promotion => {
            let promo_kind = m.promotion?;
            new_pos.board.set_piece(m.to, Some(Piece::new(piece.color, promo_kind)));
        }
        MoveKind::EnPassant => {
            new_pos.board.set_piece(m.to, Some(piece));
            // The captured pawn is on the same file as the destination,
            // same rank as the source.
            let captured_sq = Square::new(m.to.file(), m.from.rank());
            new_pos.board.set_piece(captured_sq, None);
        }
        MoveKind::Castle => {
            new_pos.board.set_piece(m.to, Some(piece));
            let rank = m.from.rank();
            let (rook_from, rook_to) = if m.to.file() > m.from.file() {
                (Square::new(7, rank), Square::new(5, rank)) // kingside castling
            } else {
                (Square::new(0, rank), Square::new(3, rank)) // queenside castling
            };
            let rook = new_pos.board.piece_at(rook_from)?;
            new_pos.board.set_piece(rook_from, None);
            new_pos.board.set_piece(rook_to, Some(rook));
        }
    }

    // --- En passant square ---
    new_pos.en_passant = if piece.kind == PieceKind::Pawn {
        let rank_diff = m.to.rank() as i8 - m.from.rank() as i8;
        if rank_diff.abs() == 2 {
            let ep_rank = (m.from.rank() as i8 + rank_diff / 2) as u8;
            Some(Square::new(m.from.file(), ep_rank))
        } else {
            None
        }
    } else {
        None
    };

    // --- Castling rights ---
    if piece.kind == PieceKind::King {
        match piece.color {
            Color::White => {
                new_pos.castling.white_kingside  = false;
                new_pos.castling.white_queenside = false;
            }
            Color::Black => {
                new_pos.castling.black_kingside  = false;
                new_pos.castling.black_queenside = false;
            }
        }
    }
    if piece.kind == PieceKind::Rook {
        match (piece.color, m.from.index()) {
            (Color::White, 0)  => new_pos.castling.white_queenside = false,
            (Color::White, 7)  => new_pos.castling.white_kingside  = false,
            (Color::Black, 56) => new_pos.castling.black_queenside = false,
            (Color::Black, 63) => new_pos.castling.black_kingside  = false,
            _ => {}
        }
    }
    // Rook captured on its starting square
    match m.to.index() {
        0  => new_pos.castling.white_queenside = false,
        7  => new_pos.castling.white_kingside  = false,
        56 => new_pos.castling.black_queenside = false,
        63 => new_pos.castling.black_kingside  = false,
        _  => {}
    }

    // --- Half-move clock ---
    if piece.kind == PieceKind::Pawn || is_capture {
        new_pos.halfmove_clock = 0;
    } else {
        new_pos.halfmove_clock = new_pos.halfmove_clock.saturating_add(1);
    }

    // --- Move number ---
    if piece.color == Color::Black {
        new_pos.fullmove_number = new_pos.fullmove_number.saturating_add(1);
    }

    // --- Side-to-move change ---
    new_pos.side_to_move = piece.color.opposite();

    Some(new_pos)
}

// ---------------------------------------------------------------------------
// Pseudo-legal generation per piece type
// ---------------------------------------------------------------------------

fn push_promotions(from: Square, to: Square, moves: &mut Vec<Move>) {
    for kind in PROMOTION_PIECES {
        moves.push(Move::promotion(from, to, kind));
    }
}

fn gen_pawn_moves(pos: &Position, sq: Square, color: Color, moves: &mut Vec<Move>) {
    let (fwd, start_rank, promo_rank): (i8, u8, u8) = match color {
        Color::White => (1, 1, 6),
        Color::Black => (-1, 6, 1),
    };
    let rank = sq.rank();

    // Single push
    if let Some(target) = offset_square(sq, 0, fwd) {
        if pos.board.is_empty(target) {
            if rank == promo_rank {
                push_promotions(sq, target, moves);
            } else {
                moves.push(Move::normal(sq, target));
                // Double push from the starting rank
                if rank == start_rank {
                    if let Some(double_target) = offset_square(sq, 0, fwd * 2) {
                        if pos.board.is_empty(double_target) {
                            moves.push(Move::normal(sq, double_target));
                        }
                    }
                }
            }
        }
    }

    // Diagonal captures + en passant
    for df in [-1i8, 1i8] {
        if let Some(target) = offset_square(sq, df, fwd) {
            if pos.board.is_occupied_by(target, color.opposite()) {
                if rank == promo_rank {
                    push_promotions(sq, target, moves);
                } else {
                    moves.push(Move::normal(sq, target));
                }
            } else if Some(target) == pos.en_passant {
                moves.push(Move::en_passant(sq, target));
            }
        }
    }
}

fn gen_knight_moves(board: &Board, sq: Square, color: Color, moves: &mut Vec<Move>) {
    for &(df, dr) in &KNIGHT_DELTAS {
        if let Some(target) = offset_square(sq, df, dr) {
            if !board.is_occupied_by(target, color) {
                moves.push(Move::normal(sq, target));
            }
        }
    }
}

fn gen_slider_moves(
    board: &Board,
    sq: Square,
    color: Color,
    dirs: &[(i8, i8)],
    moves: &mut Vec<Move>,
) {
    for &(df, dr) in dirs {
        let mut cur = sq;
        loop {
            match offset_square(cur, df, dr) {
                None => break,
                Some(target) => {
                    cur = target;
                    match board.piece_at(target) {
                        None => moves.push(Move::normal(sq, target)),
                        Some(p) if p.color == color.opposite() => {
                            moves.push(Move::normal(sq, target));
                            break;
                        }
                        Some(_) => break,
                    }
                }
            }
        }
    }
}

fn gen_king_moves(pos: &Position, sq: Square, color: Color, moves: &mut Vec<Move>) {
    let opponent = color.opposite();

    // Normal moves (1 square in every direction)
    for &(df, dr) in &KING_DELTAS {
        if let Some(target) = offset_square(sq, df, dr) {
            if !pos.board.is_occupied_by(target, color) {
                moves.push(Move::normal(sq, target));
            }
        }
    }

    // Castling — forbidden if the king is currently in check
    if is_square_attacked(&pos.board, sq, opponent) {
        return;
    }

    let rank = sq.rank();

    // Kingside castling
    let kingside = match color {
        Color::White => pos.castling.white_kingside,
        Color::Black => pos.castling.black_kingside,
    };
    if kingside {
        let transit = Square::new(5, rank);
        let dest    = Square::new(6, rank);
        if pos.board.is_empty(transit)
            && pos.board.is_empty(dest)
            && !is_square_attacked(&pos.board, transit, opponent)
            && !is_square_attacked(&pos.board, dest, opponent)
        {
            moves.push(Move::castle(sq, dest));
        }
    }

    // Queenside castling
    let queenside = match color {
        Color::White => pos.castling.white_queenside,
        Color::Black => pos.castling.black_queenside,
    };
    if queenside {
        let transit_d = Square::new(3, rank);
        let dest_c    = Square::new(2, rank);
        let b_sq      = Square::new(1, rank);
        if pos.board.is_empty(transit_d)
            && pos.board.is_empty(dest_c)
            && pos.board.is_empty(b_sq)
            && !is_square_attacked(&pos.board, transit_d, opponent)
            && !is_square_attacked(&pos.board, dest_c, opponent)
        {
            moves.push(Move::castle(sq, dest_c));
        }
    }
}

/// Visibility widened to `pub(crate)` (perf bugfix 09/07/2026, see
/// `SUIVI_PLAN_ACTION.md`) — same precedent as [`apply_move`]: reused
/// directly by `core::pgn::resolve_san_trusted`, the fast SAN resolver
/// used for bulk import (reference games database), which needs
/// pseudo-legal moves WITHOUT the king-safety filtering of
/// [`generate_legal_moves`] (which clones the position and rescans the board
/// for EACH pseudo-legal move — unnecessary when looking for only ONE
/// specific move matching an already-known SAN token).
pub(crate) fn generate_pseudo_legal(pos: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    let color = pos.side_to_move;

    // Collected first to release the immutable borrow of the board
    let pieces: Vec<_> = pos.board.pieces_of_color(color).collect();

    for (sq, piece) in pieces {
        match piece.kind {
            PieceKind::Pawn   => gen_pawn_moves(pos, sq, color, &mut moves),
            PieceKind::Knight => gen_knight_moves(&pos.board, sq, color, &mut moves),
            PieceKind::Bishop => gen_slider_moves(&pos.board, sq, color, &BISHOP_DELTAS, &mut moves),
            PieceKind::Rook   => gen_slider_moves(&pos.board, sq, color, &ROOK_DELTAS,   &mut moves),
            PieceKind::Queen  => gen_slider_moves(&pos.board, sq, color, &QUEEN_DELTAS,  &mut moves),
            PieceKind::King   => gen_king_moves(pos, sq, color, &mut moves),
        }
    }

    moves
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generates all legal moves for the side to move.
///
/// A move is legal if, after being applied, the side-to-move's king
/// is not in check.
#[must_use]
pub fn generate_legal_moves(pos: &Position) -> Vec<Move> {
    let color = pos.side_to_move;
    generate_pseudo_legal(pos)
        .into_iter()
        .filter(|&m| {
            // apply_move only returns None if the position is inconsistent
            // (should not happen with valid pseudo-legal moves)
            let Some(new_pos) = apply_move(pos, m) else { return false; };
            new_pos
                .board
                .find_king(color)
                .is_some_and(|king_sq| !is_square_attacked(&new_pos.board, king_sq, color.opposite()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Position;

    fn pos(fen: &str) -> Position {
        Position::from_fen(fen).expect("FEN invalide dans le test")
    }

    fn sq(alg: &str) -> Square {
        Square::from_algebraic(alg).expect("case invalide dans le test")
    }

    // --- Starting position ---

    #[test]
    fn test_starting_position_20_moves() {
        let p = Position::starting();
        let moves = generate_legal_moves(&p);
        assert_eq!(moves.len(), 20, "La position de départ doit avoir 20 coups légaux");
    }

    #[test]
    fn test_starting_position_not_in_check() {
        assert!(!is_in_check(&Position::starting()));
    }

    // --- Pawns ---

    #[test]
    fn test_pawn_double_push_from_start() {
        let p = Position::starting();
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::normal(sq("e2"), sq("e4"))));
        assert!(moves.contains(&Move::normal(sq("e2"), sq("e3"))));
    }

    #[test]
    fn test_pawn_blocked_no_push() {
        // White rook on e3 blocks the pawn on e2
        let p = pos("8/8/8/8/8/4R3/4P3/4K3 w - - 0 1");
        let moves = generate_legal_moves(&p);
        // Pawn e2 cannot advance
        assert!(!moves.contains(&Move::normal(sq("e2"), sq("e3"))));
        assert!(!moves.contains(&Move::normal(sq("e2"), sq("e4"))));
    }

    #[test]
    fn test_pawn_blocked_double_push_when_single_blocked() {
        // Piece on e3: the double push from e2 is also blocked
        let p = pos("8/8/8/8/8/4r3/4P3/4K3 w - - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(!moves.contains(&Move::normal(sq("e2"), sq("e4"))));
    }

    #[test]
    fn test_pawn_capture() {
        let p = pos("8/8/8/8/8/3p4/4P3/4K3 w - - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::normal(sq("e2"), sq("d3"))));
    }

    #[test]
    fn test_pawn_promotion() {
        // White pawn on e7, promotion possible
        let p = pos("8/4P3/8/8/8/8/8/4K3 w - - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::promotion(sq("e7"), sq("e8"), PieceKind::Queen)));
        assert!(moves.contains(&Move::promotion(sq("e7"), sq("e8"), PieceKind::Rook)));
        assert!(moves.contains(&Move::promotion(sq("e7"), sq("e8"), PieceKind::Bishop)));
        assert!(moves.contains(&Move::promotion(sq("e7"), sq("e8"), PieceKind::Knight)));
    }

    #[test]
    fn test_en_passant_generated() {
        // White pawn e5, black just played d7d5 → en passant available on d6
        let p = pos("rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3");
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::en_passant(sq("e5"), sq("d6"))));
    }

    // --- Knights ---

    #[test]
    fn test_knight_moves_from_start() {
        let p = Position::starting();
        let moves = generate_legal_moves(&p);
        // Knight b1
        assert!(moves.contains(&Move::normal(sq("b1"), sq("a3"))));
        assert!(moves.contains(&Move::normal(sq("b1"), sq("c3"))));
        // Knight g1
        assert!(moves.contains(&Move::normal(sq("g1"), sq("f3"))));
        assert!(moves.contains(&Move::normal(sq("g1"), sq("h3"))));
    }

    #[test]
    fn test_knight_center_has_8_moves() {
        // Lone white knight in the center, white king on a1
        let p = pos("8/8/8/8/4N3/8/8/K7 w - - 0 1");
        let knight_moves: Vec<_> = generate_legal_moves(&p)
            .into_iter()
            .filter(|m| m.from == sq("e4"))
            .collect();
        assert_eq!(knight_moves.len(), 8);
    }

    // --- Sliders ---

    #[test]
    fn test_rook_open_file() {
        // White rook a1, white king h1
        let p = pos("8/8/8/8/8/8/8/R6K w - - 0 1");
        let rook_moves: Vec<_> = generate_legal_moves(&p)
            .into_iter()
            .filter(|m| m.from == sq("a1"))
            .collect();
        // Rook can go to a2..a8 (7) and b1..g1 (6) = 13
        assert_eq!(rook_moves.len(), 13);
    }

    #[test]
    fn test_bishop_open_diagonal() {
        // White bishop c1, white king a1
        let p = pos("8/8/8/8/8/8/8/K1B5 w - - 0 1");
        let bishop_moves: Vec<_> = generate_legal_moves(&p)
            .into_iter()
            .filter(|m| m.from == sq("c1"))
            .collect();
        // Diagonal c1-h6 (5 squares) + c1-b2, a3 (but a3 is not... wait)
        // c1 diagonals: b2-a3 (2 squares), d2-e3-f4-g5-h6 (5 squares) = 7
        assert_eq!(bishop_moves.len(), 7);
    }

    // --- Castling ---

    #[test]
    fn test_castling_kingside_white() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::castle(sq("e1"), sq("g1"))));
    }

    #[test]
    fn test_castling_queenside_white() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(moves.contains(&Move::castle(sq("e1"), sq("c1"))));
    }

    #[test]
    fn test_castling_blocked_by_piece() {
        // Bishop on f1 blocks white's kingside castling
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3KB1R w KQkq - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(!moves.contains(&Move::castle(sq("e1"), sq("g1"))));
    }

    #[test]
    fn test_castling_blocked_by_check() {
        // Black rook on e8 gives check on e1 → castling not possible
        let p = pos("4r3/8/8/8/8/8/8/R3K2R w KQ - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(!moves.contains(&Move::castle(sq("e1"), sq("g1"))));
        assert!(!moves.contains(&Move::castle(sq("e1"), sq("c1"))));
    }

    #[test]
    fn test_castling_not_through_attacked_square() {
        // Black rook on f8 attacks f1 → no white kingside castling
        let p = pos("5r2/8/8/8/8/8/8/R3K2R w KQ - 0 1");
        let moves = generate_legal_moves(&p);
        assert!(!moves.contains(&Move::castle(sq("e1"), sq("g1"))));
    }

    // --- Pinned piece ---

    #[test]
    fn test_pinned_rook_can_only_stay_on_file() {
        // White king e1, white rook e4, black rook e8 → vertical pin
        let p = pos("4r3/8/8/8/4R3/8/8/4K3 w - - 0 1");
        let rook_moves: Vec<_> = generate_legal_moves(&p)
            .into_iter()
            .filter(|m| m.from == sq("e4"))
            .collect();
        // The rook can only move on the e-file
        for m in &rook_moves {
            assert_eq!(m.to.file(), sq("e4").file(),
                "Tour clouée : déplacement hors de la colonne e détecté vers {}", m.to);
        }
    }

    // --- Check ---

    #[test]
    fn test_in_check_detection() {
        // White king e1 in check by black queen e8
        let p = pos("4q3/8/8/8/8/8/8/4K3 w - - 0 1");
        assert!(is_in_check(&p));
    }

    #[test]
    fn test_not_in_check_starting_position() {
        assert!(!is_in_check(&Position::starting()));
    }

    #[test]
    fn test_in_check_only_legal_moves_escape() {
        // White king e1 in check by black queen e8, only escape: move the king
        let p = pos("4q3/8/8/8/8/8/8/4K3 w - - 0 1");
        let moves = generate_legal_moves(&p);
        // All legal moves must resolve the check
        for m in &moves {
            let new_pos = apply_move(&p, *m).expect("apply_move valide sur coup légal");
            assert!(!is_in_check(&new_pos));
        }
    }

    // --- Checkmate and stalemate ---

    #[test]
    fn test_checkmate_fools_mate() {
        // Fool's mate: 1.f3 e5 2.g4 Qh4#
        let p = pos("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3");
        assert!(is_in_check(&p));
        assert_eq!(generate_legal_moves(&p).len(), 0, "Mat : aucun coup légal attendu");
    }

    #[test]
    fn test_stalemate_no_moves() {
        // Stalemate: black king on h8, white queen f7, white king g6
        let p = pos("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
        assert!(!is_in_check(&p), "Pat : le roi ne doit pas être en échec");
        assert_eq!(generate_legal_moves(&p).len(), 0, "Pat : aucun coup légal attendu");
    }

    // --- Square attack ---

    #[test]
    fn test_is_square_attacked_by_pawn() {
        let mut board = Board::empty();
        board.set_piece(sq("e2"), Some(Piece::new(Color::White, PieceKind::Pawn)));
        assert!(is_square_attacked(&board, sq("d3"), Color::White));
        assert!(is_square_attacked(&board, sq("f3"), Color::White));
        assert!(!is_square_attacked(&board, sq("e3"), Color::White)); // push, not an attack
        assert!(!is_square_attacked(&board, sq("d2"), Color::White));
    }

    #[test]
    fn test_is_square_attacked_by_knight() {
        let mut board = Board::empty();
        board.set_piece(sq("d4"), Some(Piece::new(Color::Black, PieceKind::Knight)));
        assert!(is_square_attacked(&board, sq("e6"), Color::Black));
        assert!(is_square_attacked(&board, sq("f5"), Color::Black));
        assert!(!is_square_attacked(&board, sq("d5"), Color::Black));
    }

    #[test]
    fn test_is_square_attacked_through_pieces_blocked() {
        // White rook a1, white piece a4 blocks the attack on a8
        let mut board = Board::empty();
        board.set_piece(sq("a1"), Some(Piece::new(Color::White, PieceKind::Rook)));
        board.set_piece(sq("a4"), Some(Piece::new(Color::White, PieceKind::Pawn)));
        assert!(!is_square_attacked(&board, sq("a8"), Color::White));
        assert!(is_square_attacked(&board, sq("a3"), Color::White));
    }
}
