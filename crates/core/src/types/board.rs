use super::{
    piece::{Color, Piece, PieceKind},
    square::Square,
};

/// Board representation: 64 squares, each empty or occupied by a piece.
///
/// `Board` focuses on **piece placement** only.
/// Position metadata (side to move, castling, en passant…) is in [`super::position::Position`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    squares: [Option<Piece>; 64],
}

impl Board {
    /// Empty board (no pieces).
    #[must_use]
    pub fn empty() -> Self {
        Self { squares: [None; 64] }
    }

    // -----------------------------------------------------------------------
    // Square access
    // -----------------------------------------------------------------------

    /// Piece on a square, or `None` if empty.
    #[must_use]
    #[inline]
    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        self.squares[sq.index() as usize]
    }

    /// Places (or removes) a piece on a square.
    #[inline]
    pub fn set_piece(&mut self, sq: Square, piece: Option<Piece>) {
        self.squares[sq.index() as usize] = piece;
    }

    /// Is the square empty?
    #[must_use]
    #[inline]
    pub fn is_empty(&self, sq: Square) -> bool {
        self.squares[sq.index() as usize].is_none()
    }

    /// Is the square occupied by a piece of the given color?
    #[must_use]
    #[inline]
    pub fn is_occupied_by(&self, sq: Square, color: Color) -> bool {
        self.squares[sq.index() as usize].is_some_and(|p| p.color == color)
    }

    // -----------------------------------------------------------------------
    // Iteration
    // -----------------------------------------------------------------------

    /// Iterates over all pieces on the board: `(Square, Piece)`.
    pub fn pieces(&self) -> impl Iterator<Item = (Square, Piece)> + '_ {
        self.squares
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                #[allow(clippy::cast_possible_truncation)]
                slot.map(|piece| (Square::from_index(i as u8), piece))
            })
    }

    /// Iterates over all pieces of a given color.
    pub fn pieces_of_color(&self, color: Color) -> impl Iterator<Item = (Square, Piece)> + '_ {
        self.pieces().filter(move |(_, p)| p.color == color)
    }

    /// Iterates over all pieces of a given type and color.
    pub fn pieces_of_kind(
        &self,
        color: Color,
        kind: PieceKind,
    ) -> impl Iterator<Item = (Square, Piece)> + '_ {
        self.pieces()
            .filter(move |(_, p)| p.color == color && p.kind == kind)
    }

    // -----------------------------------------------------------------------
    // Utility queries
    // -----------------------------------------------------------------------

    /// Finds the square of the king of a given color.
    ///
    /// Returns `None` if the king is absent (invalid position).
    #[must_use]
    pub fn find_king(&self, color: Color) -> Option<Square> {
        self.pieces_of_kind(color, PieceKind::King)
            .next()
            .map(|(sq, _)| sq)
    }

    /// Number of pieces of a given type and color.
    #[must_use]
    pub fn piece_count(&self, color: Color, kind: PieceKind) -> usize {
        self.pieces_of_kind(color, kind).count()
    }

    /// Total number of pieces on the board.
    #[must_use]
    pub fn total_pieces(&self) -> usize {
        self.squares.iter().filter(|s| s.is_some()).count()
    }

    /// Total number of pieces of a given color.
    #[must_use]
    pub fn total_pieces_of_color(&self, color: Color) -> usize {
        self.pieces_of_color(color).count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(alg: &str) -> Square {
        Square::from_algebraic(alg).unwrap()
    }

    #[test]
    fn test_empty_board() {
        let board = Board::empty();
        for i in 0u8..64 {
            assert!(board.piece_at(Square::from_index(i)).is_none());
        }
        assert_eq!(board.total_pieces(), 0);
    }

    #[test]
    fn test_set_and_get_piece() {
        let mut board = Board::empty();
        let piece = Piece::new(Color::White, PieceKind::Queen);
        board.set_piece(sq("d1"), Some(piece));
        assert_eq!(board.piece_at(sq("d1")), Some(piece));
        assert!(board.is_empty(sq("e1")));
    }

    #[test]
    fn test_is_occupied_by() {
        let mut board = Board::empty();
        board.set_piece(sq("e1"), Some(Piece::new(Color::White, PieceKind::King)));
        assert!(board.is_occupied_by(sq("e1"), Color::White));
        assert!(!board.is_occupied_by(sq("e1"), Color::Black));
        assert!(!board.is_occupied_by(sq("e2"), Color::White));
    }

    #[test]
    fn test_find_king() {
        let mut board = Board::empty();
        board.set_piece(sq("e1"), Some(Piece::new(Color::White, PieceKind::King)));
        board.set_piece(sq("e8"), Some(Piece::new(Color::Black, PieceKind::King)));
        assert_eq!(board.find_king(Color::White), Some(sq("e1")));
        assert_eq!(board.find_king(Color::Black), Some(sq("e8")));
    }

    #[test]
    fn test_find_king_absent() {
        let board = Board::empty();
        assert!(board.find_king(Color::White).is_none());
    }

    #[test]
    fn test_piece_count() {
        let mut board = Board::empty();
        board.set_piece(sq("a1"), Some(Piece::new(Color::White, PieceKind::Rook)));
        board.set_piece(sq("h1"), Some(Piece::new(Color::White, PieceKind::Rook)));
        board.set_piece(sq("a8"), Some(Piece::new(Color::Black, PieceKind::Rook)));
        assert_eq!(board.piece_count(Color::White, PieceKind::Rook), 2);
        assert_eq!(board.piece_count(Color::Black, PieceKind::Rook), 1);
        assert_eq!(board.piece_count(Color::White, PieceKind::Queen), 0);
    }

    #[test]
    fn test_total_pieces() {
        let mut board = Board::empty();
        board.set_piece(sq("e1"), Some(Piece::new(Color::White, PieceKind::King)));
        board.set_piece(sq("e8"), Some(Piece::new(Color::Black, PieceKind::King)));
        board.set_piece(sq("d1"), Some(Piece::new(Color::White, PieceKind::Queen)));
        assert_eq!(board.total_pieces(), 3);
        assert_eq!(board.total_pieces_of_color(Color::White), 2);
        assert_eq!(board.total_pieces_of_color(Color::Black), 1);
    }

    #[test]
    fn test_pieces_iterator() {
        let mut board = Board::empty();
        board.set_piece(sq("a1"), Some(Piece::new(Color::White, PieceKind::Rook)));
        board.set_piece(sq("h8"), Some(Piece::new(Color::Black, PieceKind::Rook)));
        let all: Vec<_> = board.pieces().collect();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_remove_piece() {
        let mut board = Board::empty();
        board.set_piece(sq("e1"), Some(Piece::new(Color::White, PieceKind::King)));
        board.set_piece(sq("e1"), None);
        assert!(board.is_empty(sq("e1")));
        assert_eq!(board.total_pieces(), 0);
    }
}
