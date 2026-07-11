//! SCID move decoder — shared between si4 and si5 (the move format is
//! bit-for-bit identical between the two versions, see `si5_specification_fr.txt`
//! §4.5.1 and the historical note at the top of that same document).
//!
//! Ported directly from `bytebuf.h::decodeMove` and `game.cpp::decodeMove`
//! of the provided SCID source tree. The numeric tables below are
//! exact copies of the corresponding C++ tables — deliberately not
//! "simplified" so they remain verifiable line-by-line against the source.

use crate::bytes::BeReader;
use crate::error::GameDecodeError;
use chess_core::types::piece::{Color, PieceKind};
use chess_core::types::square::Square;

/// Table of the 16 slots (piece index -> square) for one side, in the
/// standard starting position. Ported from `Position::getStdStart()`
/// (`position.cpp`):
///   0=King, 1=Rook(a), 2=Knight(b), 3=Bishop(c), 4=Queen(d),
///   5=Bishop(f), 6=Knight(g), 7=Rook(h), 8..15=Pawns a..h.
#[must_use]
pub fn standard_piece_list(color: Color) -> [Square; 16] {
    let back_rank = if color == Color::White { 0 } else { 7 };
    let pawn_rank = if color == Color::White { 1 } else { 6 };
    [
        Square::new(4, back_rank), // 0: King (e)
        Square::new(0, back_rank), // 1: Rook (a)
        Square::new(1, back_rank), // 2: Knight (b)
        Square::new(2, back_rank), // 3: Bishop (c)
        Square::new(3, back_rank), // 4: Queen (d)
        Square::new(5, back_rank), // 5: Bishop (f)
        Square::new(6, back_rank), // 6: Knight (g)
        Square::new(7, back_rank), // 7: Rook (h)
        Square::new(0, pawn_rank), // 8: Pawn a
        Square::new(1, pawn_rank), // 9: Pawn b
        Square::new(2, pawn_rank), // 10: Pawn c
        Square::new(3, pawn_rank), // 11: Pawn d
        Square::new(4, pawn_rank), // 12: Pawn e
        Square::new(5, pawn_rank), // 13: Pawn f
        Square::new(6, pawn_rank), // 14: Pawn g
        Square::new(7, pawn_rank), // 15: Pawn h
    ]
}

/// Origin and destination squares of the Rook involved in castling (Rook
/// a/h -> d/f, on the side's back rank). Returns SQUARES, not a
/// `list` index: the real index (previously assumed fixed: 1=Rook a,
/// 7=Rook h) must be looked up dynamically by searching for the origin
/// square in `list[side][0..count[side]]` (see `game_blob::apply_one_move`)
/// — fixed on 12/07/2026 (V2 Phase C2, task #22) because this fixed-index
/// assumption is false for a non-standard starting position
/// (`build_piece_lists` does not assign any fixed index to the Rooks), and was
/// in any case only guaranteed in the standard position AS LONG AS no
/// capture had been able to reassign this index via the "swap with the last
/// active element" mechanism (see the 12/07/2026 bugfix on renumbering).
#[must_use]
pub fn castling_rook(color: Color, kingside: bool) -> (Square, Square) {
    let back_rank = if color == Color::White { 0 } else { 7 };
    if kingside {
        (Square::new(7, back_rank), Square::new(5, back_rank)) // Rook h -> f
    } else {
        (Square::new(0, back_rank), Square::new(3, back_rank)) // Rook a -> d
    }
}

/// Result of decoding a move byte (or byte pair), BEFORE
/// resolution into a `chess_core` legal move (see `game_blob.rs`).
#[derive(Debug, Clone, Copy)]
pub enum DecodedMove {
    /// Normal move (or capture), with an optional promotion.
    Normal { to: Square, promotion: Option<PieceKind> },
    /// Kingside castling (king e1/e8 -> g1/g8).
    CastleKingside,
    /// Queenside castling (king e1/e8 -> c1/c8).
    CastleQueenside,
    /// Null move ("null move"): not representable in `chess_core`.
    NullMove,
}

/// Decodes a move byte (the low 4 bits, `code`) given the type and the
/// origin square of the piece being moved (determined by the caller from
/// the current position — see `game_blob.rs`).
///
/// `cursor` allows reading the extra byte for the special "diagonal Queen,
/// 2 bytes" case (the only case where a move occupies more than one byte).
///
/// # Errors
/// [`GameDecodeError::BadMoveStream`] if the code does not match any
/// valid move for this piece type, or if the resulting square is off the board.
pub fn decode_move(
    color: Color,
    moving_piece: PieceKind,
    from: Square,
    raw_byte: u8,
    cursor: &mut BeReader<'_>,
) -> Result<DecodedMove, GameDecodeError> {
    let code = i32::from(raw_byte & 0x0F);
    let from_idx = i32::from(from.index());

    let to_idx: i32 = match moving_piece {
        PieceKind::Pawn => {
            const PROMO: [Option<PieceKind>; 16] = [
                None, None, None,
                Some(PieceKind::Queen), Some(PieceKind::Queen), Some(PieceKind::Queen),
                Some(PieceKind::Rook), Some(PieceKind::Rook), Some(PieceKind::Rook),
                Some(PieceKind::Bishop), Some(PieceKind::Bishop), Some(PieceKind::Bishop),
                Some(PieceKind::Knight), Some(PieceKind::Knight), Some(PieceKind::Knight),
                None,
            ];
            const SQDIFF: [i32; 16] = [7, 8, 9, 7, 8, 9, 7, 8, 9, 7, 8, 9, 7, 8, 9, 16];
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let idx = code as usize;
            let diff = SQDIFF[idx];
            let to = if color == Color::White { from_idx + diff } else { from_idx - diff };
            return finish_normal(to, PROMO[idx]);
        }
        PieceKind::Knight => {
            const SQDIFF: [i32; 16] = [0, -17, -15, -10, -6, 6, 10, 15, 17, 0, 0, 0, 0, 0, 0, 0];
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let idx = code as usize;
            from_idx + SQDIFF[idx]
        }
        PieceKind::Bishop => {
            let fylediff = code_low3(code) - i32::from(from.file());
            if code >= 8 { from_idx - 7 * fylediff } else { from_idx + 9 * fylediff }
        }
        PieceKind::Queen => {
            if code == i32::from(from.file()) {
                // Diagonal move over 2 bytes (the only multi-byte move case).
                let byte2 = cursor
                    .read_u8()
                    .map_err(|_| GameDecodeError::BadMoveStream("octet manquant (Dame diagonale)"))?;
                let to = i32::from(byte2) - 64;
                return finish_normal(to, None);
            }
            rook_like(from, code)
        }
        PieceKind::Rook => rook_like(from, code),
        PieceKind::King => {
            if code == 0 {
                return Ok(DecodedMove::NullMove);
            }
            if code <= 8 {
                const SQDIFF: [i32; 9] = [0, -9, -8, -7, -1, 1, 7, 8, 9];
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let idx = code as usize;
                from_idx + SQDIFF[idx]
            } else if code == 9 {
                return Ok(DecodedMove::CastleQueenside);
            } else if code == 10 {
                return Ok(DecodedMove::CastleKingside);
            } else {
                return Err(GameDecodeError::BadMoveStream("code de Roi invalide (11-15)"));
            }
        }
    };

    finish_normal(to_idx, None)
}

/// Equivalent of the source code's `square_Fyle(moveCode)`: the low 3 bits
/// of `code` (0-15), treated as if it were a square.
fn code_low3(code: i32) -> i32 {
    code & 7
}

/// Common Rook / Queen encoding ("rook-like" movement).
fn rook_like(from: Square, code: i32) -> i32 {
    if code >= 8 {
        // Vertical move: square_Make(fyle(from), code-8)
        ((code - 8) << 3) | i32::from(from.file())
    } else {
        // Horizontal move: square_Make(code, rank(from))
        (i32::from(from.rank()) << 3) | code
    }
}

fn finish_normal(to_idx: i32, promotion: Option<PieceKind>) -> Result<DecodedMove, GameDecodeError> {
    if !(0..64).contains(&to_idx) {
        return Err(GameDecodeError::BadMoveStream("case d'arrivée hors échiquier"));
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let to = Square::from_index(to_idx as u8);
    Ok(DecodedMove::Normal { to, promotion })
}
