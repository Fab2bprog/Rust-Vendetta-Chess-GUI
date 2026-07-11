use super::{
    board::Board,
    piece::{Color, Piece, PieceKind},
    square::Square,
};

/// Castling rights for both colors.
///
/// Clippy (04/07/2026): deliberate `#[allow(struct_excessive_bools)]` — 4
/// named fields each corresponding to a distinct FEN right (`K`/`Q`/`k`/`q`),
/// not a grab-bag of unrelated booleans; refactoring into an enum/bitflags
/// would be disproportionate for a cosmetic lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct CastlingRights {
    pub white_kingside:  bool,
    pub white_queenside: bool,
    pub black_kingside:  bool,
    pub black_queenside: bool,
}

impl CastlingRights {
    /// All castling rights allowed.
    #[must_use]
    pub fn all() -> Self {
        Self {
            white_kingside:  true,
            white_queenside: true,
            black_kingside:  true,
            black_queenside: true,
        }
    }

    /// No castling rights allowed.
    #[must_use]
    pub fn none() -> Self {
        Self {
            white_kingside:  false,
            white_queenside: false,
            black_kingside:  false,
            black_queenside: false,
        }
    }

    /// FEN string (`"KQkq"`, `"Kq"`, `"-"` …).
    #[must_use]
    pub fn to_fen(self) -> String {
        let mut s = String::new();
        if self.white_kingside  { s.push('K'); }
        if self.white_queenside { s.push('Q'); }
        if self.black_kingside  { s.push('k'); }
        if self.black_queenside { s.push('q'); }
        if s.is_empty() { s.push('-'); }
        s
    }
}

impl Default for CastlingRights {
    fn default() -> Self {
        Self::all()
    }
}

// ---------------------------------------------------------------------------

/// A complete chess position (in the FEN sense).
///
/// Contains a [`Board`] (piece placement) and the game
/// metadata: side to move, castling rights, en passant, clocks.
#[derive(Debug, Clone)]
pub struct Position {
    /// Piece placement.
    pub board:           Board,
    /// Side to move.
    pub side_to_move:    Color,
    /// Castling rights.
    pub castling:        CastlingRights,
    /// En passant square (if available).
    pub en_passant:      Option<Square>,
    /// Half-moves since the last capture or pawn advance (50-move rule).
    pub halfmove_clock:  u8,
    /// Full move number (starts at 1).
    pub fullmove_number: u16,
}

impl Position {
    /// Standard starting position.
    #[must_use]
    pub fn starting() -> Self {
        let mut board = Board::empty();

        let back_rank = [
            PieceKind::Rook,
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Queen,
            PieceKind::King,
            PieceKind::Bishop,
            PieceKind::Knight,
            PieceKind::Rook,
        ];

        for (file, &kind) in back_rank.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let f = file as u8;
            board.set_piece(Square::new(f, 0), Some(Piece::new(Color::White, kind)));
            board.set_piece(Square::new(f, 7), Some(Piece::new(Color::Black, kind)));
            board.set_piece(Square::new(f, 1), Some(Piece::new(Color::White, PieceKind::Pawn)));
            board.set_piece(Square::new(f, 6), Some(Piece::new(Color::Black, PieceKind::Pawn)));
        }

        Self {
            board,
            side_to_move:    Color::White,
            castling:        CastlingRights::all(),
            en_passant:      None,
            halfmove_clock:  0,
            fullmove_number: 1,
        }
    }

    /// Piece on a square (delegates to `Board`).
    #[must_use]
    #[inline]
    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        self.board.piece_at(sq)
    }

    /// Places or removes a piece (delegates to `Board`).
    #[inline]
    pub fn set_piece(&mut self, sq: Square, piece: Option<Piece>) {
        self.board.set_piece(sq, piece);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starting_position_pieces() {
        let pos = Position::starting();

        let e1 = Square::from_algebraic("e1").unwrap();
        assert_eq!(pos.piece_at(e1), Some(Piece::new(Color::White, PieceKind::King)));

        let e8 = Square::from_algebraic("e8").unwrap();
        assert_eq!(pos.piece_at(e8), Some(Piece::new(Color::Black, PieceKind::King)));

        let e4 = Square::from_algebraic("e4").unwrap();
        assert!(pos.piece_at(e4).is_none());
    }

    #[test]
    fn test_starting_position_metadata() {
        let pos = Position::starting();
        assert_eq!(pos.side_to_move, Color::White);
        assert_eq!(pos.halfmove_clock, 0);
        assert_eq!(pos.fullmove_number, 1);
        assert!(pos.en_passant.is_none());
        assert!(pos.castling.white_kingside);
        assert!(pos.castling.black_queenside);
    }

    #[test]
    fn test_starting_position_pawn_count() {
        let pos = Position::starting();
        assert_eq!(pos.board.piece_count(Color::White, PieceKind::Pawn), 8);
        assert_eq!(pos.board.piece_count(Color::Black, PieceKind::Pawn), 8);
    }

    #[test]
    fn test_starting_position_total_pieces() {
        let pos = Position::starting();
        assert_eq!(pos.board.total_pieces(), 32);
        assert_eq!(pos.board.total_pieces_of_color(Color::White), 16);
        assert_eq!(pos.board.total_pieces_of_color(Color::Black), 16);
    }

    #[test]
    fn test_starting_king_found() {
        let pos = Position::starting();
        let wk = pos.board.find_king(Color::White).unwrap();
        let bk = pos.board.find_king(Color::Black).unwrap();
        assert_eq!(wk, Square::from_algebraic("e1").unwrap());
        assert_eq!(bk, Square::from_algebraic("e8").unwrap());
    }

    #[test]
    fn test_set_and_get_piece() {
        let mut pos = Position::starting();
        let e4 = Square::from_algebraic("e4").unwrap();
        pos.set_piece(e4, Some(Piece::new(Color::White, PieceKind::Rook)));
        assert_eq!(pos.piece_at(e4), Some(Piece::new(Color::White, PieceKind::Rook)));
        pos.set_piece(e4, None);
        assert!(pos.piece_at(e4).is_none());
    }

    #[test]
    fn test_castling_rights_fen() {
        assert_eq!(CastlingRights::all().to_fen(),  "KQkq");
        assert_eq!(CastlingRights::none().to_fen(), "-");
    }
}
