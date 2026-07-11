//! Standard Algebraic Notation (SAN) for chess moves.
//!
//! SAN (Standard Algebraic Notation) is the format used in PGN files.
//! Examples: `"e4"`, `"Nf3"`, `"O-O"`, `"exd5"`, `"e8=Q"`, `"Nbd2"`, `"Rxe1#"`.

#![allow(clippy::cast_possible_truncation)]

use crate::{
    movegen::{apply_move, generate_legal_moves, is_in_check},
    types::{
        chess_move::{Move, MoveKind},
        piece::PieceKind,
        position::Position,
    },
};

// ---------------------------------------------------------------------------
// SAN generation
// ---------------------------------------------------------------------------

/// Converts a move to SAN notation for the given position.
///
/// The move must be legal in the position — no verification is performed.
#[must_use]
pub fn move_to_san(pos: &Position, m: Move) -> String {
    // --- Castling ---
    if m.kind == MoveKind::Castle {
        let base = if m.to.file() > m.from.file() { "O-O" } else { "O-O-O" };
        return format!("{base}{}", check_suffix(pos, m));
    }

    let Some(piece) = pos.board.piece_at(m.from) else {
        return m.to_uci(); // fallback (should not happen)
    };

    let mut san = String::new();

    // --- Piece letter (not for pawns) ---
    if piece.kind != PieceKind::Pawn {
        san.push(piece.kind.fen_char().to_ascii_uppercase());
    }

    // --- Disambiguation ---
    if piece.kind != PieceKind::Pawn {
        // Other pieces of the same type able to reach the same square
        let ambiguous: Vec<Move> = generate_legal_moves(pos)
            .into_iter()
            .filter(|&other| {
                other != m
                    && other.to == m.to
                    && pos
                        .board
                        .piece_at(other.from)
                        .is_some_and(|p| p.kind == piece.kind)
            })
            .collect();

        if !ambiguous.is_empty() {
            let conflict_file = ambiguous.iter().any(|a| a.from.file() == m.from.file());
            let conflict_rank = ambiguous.iter().any(|a| a.from.rank() == m.from.rank());

            // Rule: file is enough if no ambiguous piece on the same file;
            //       otherwise rank is enough if no ambiguous piece on the same rank;
            //       otherwise both.
            if !conflict_file {
                san.push((b'a' + m.from.file()) as char);
            } else if !conflict_rank {
                san.push((b'1' + m.from.rank()) as char);
            } else {
                san.push((b'a' + m.from.file()) as char);
                san.push((b'1' + m.from.rank()) as char);
            }
        }
    }

    // --- Capture ---
    let is_capture = pos.board.piece_at(m.to).is_some() || m.kind == MoveKind::EnPassant;
    if is_capture {
        if piece.kind == PieceKind::Pawn {
            san.push((b'a' + m.from.file()) as char);
        }
        san.push('x');
    }

    // --- Destination square ---
    san.push_str(&m.to.to_algebraic());

    // --- Promotion ---
    if let Some(promo) = m.promotion {
        san.push('=');
        san.push(promo.fen_char().to_ascii_uppercase());
    }

    // --- Check / checkmate suffix ---
    san.push_str(check_suffix(pos, m));

    san
}

/// Returns `"+"` if the move gives check, `"#"` if mate, `""` otherwise.
fn check_suffix(pos: &Position, m: Move) -> &'static str {
    let Some(new_pos) = apply_move(pos, m) else { return ""; };
    if is_in_check(&new_pos) {
        let legal = generate_legal_moves(&new_pos);
        if legal.is_empty() { "#" } else { "+" }
    } else {
        ""
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{chess_move::Move, square::Square, Position};

    fn pos(fen: &str) -> Position {
        Position::from_fen(fen).expect("FEN invalide")
    }

    fn sq(alg: &str) -> Square {
        Square::from_algebraic(alg).expect("case invalide")
    }

    #[test]
    fn test_san_pawn_push() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e4"));
        assert_eq!(move_to_san(&p, m), "e4");
    }

    #[test]
    fn test_san_pawn_single_push() {
        let p = Position::starting();
        let m = Move::normal(sq("e2"), sq("e3"));
        assert_eq!(move_to_san(&p, m), "e3");
    }

    #[test]
    fn test_san_knight_move() {
        let p = Position::starting();
        let m = Move::normal(sq("g1"), sq("f3"));
        assert_eq!(move_to_san(&p, m), "Nf3");
    }

    #[test]
    fn test_san_pawn_capture() {
        let p = pos("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2");
        let m = Move::normal(sq("e4"), sq("d5"));
        assert_eq!(move_to_san(&p, m), "exd5");
    }

    #[test]
    fn test_san_piece_capture() {
        // White knight on e3, black pawn on d5 → Nxd5
        // e3 = (file=4, rank=2), d5 = (file=3, rank=4), delta=(-1,+2): valid knight move
        let p = pos("4k3/8/8/3p4/8/4N3/8/4K3 w - - 0 1");
        let m = Move::normal(sq("e3"), sq("d5"));
        assert_eq!(move_to_san(&p, m), "Nxd5");
    }

    #[test]
    fn test_san_castling_kingside() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let m = Move::castle(sq("e1"), sq("g1"));
        assert_eq!(move_to_san(&p, m), "O-O");
    }

    #[test]
    fn test_san_castling_queenside() {
        let p = pos("r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R w KQkq - 0 1");
        let m = Move::castle(sq("e1"), sq("c1"));
        assert_eq!(move_to_san(&p, m), "O-O-O");
    }

    #[test]
    fn test_san_promotion() {
        let p = pos("8/4P3/8/8/8/8/8/4K3 w - - 0 1");
        let m = Move::promotion(sq("e7"), sq("e8"), PieceKind::Queen);
        assert_eq!(move_to_san(&p, m), "e8=Q");
    }

    #[test]
    fn test_san_check_suffix() {
        // White queen d1→e2: same file as black king (e8), clear path → check
        // Position: white king e1, white queen d1, black king e8
        let p = pos("4k3/8/8/8/8/8/8/3QK3 w - - 0 1");
        let m = Move::normal(sq("d1"), sq("e2"));
        let san = move_to_san(&p, m);
        assert!(san.ends_with('+'), "Attendu suffixe '+', obtenu: {san}");
        assert_eq!(san, "Qe2+");
    }

    #[test]
    fn test_san_checkmate_suffix() {
        // Fool's mate: after 1.f3 e5 2.g4 Qh4#
        // FEN after 2.g4 (black to move) — black queen on d8 (rnbqkbnr)
        let p = pos("rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq g3 0 2");
        let m = Move::normal(sq("d8"), sq("h4"));
        let san = move_to_san(&p, m);
        assert!(san.ends_with('#'), "Attendu suffixe '#', obtenu: {san}");
        assert_eq!(san, "Qh4#");
    }

    #[test]
    fn test_san_disambiguation_by_file() {
        // Two white rooks on the same rank both able to reach d1
        // White king on e2 (not on rank 1) → clear path for Ra1 and Rh1
        let p = pos("4k3/8/8/8/8/8/4K3/R6R w - - 0 1");
        // Rook a1 → d1 (disambiguation by file: 'a')
        let m = Move::normal(sq("a1"), sq("d1"));
        let san = move_to_san(&p, m);
        assert_eq!(san, "Rad1");
    }

    #[test]
    fn test_san_disambiguation_by_rank() {
        // Two white rooks on the same file (a) both able to reach a4
        // Ra3 and Ra6, white king e2, black king e8
        let p = pos("4k3/8/R7/8/8/R7/4K3/8 w - - 0 1");
        // Rook a3 → a4 (disambiguation by rank: '3')
        let m = Move::normal(sq("a3"), sq("a4"));
        let san = move_to_san(&p, m);
        assert_eq!(san, "R3a4");
    }
}
