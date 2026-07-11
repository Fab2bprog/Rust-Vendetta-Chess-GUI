/// Color of a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    /// Returns the opposite color.
    #[must_use]
    #[inline]
    pub fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::White => write!(f, "Blanc"),
            Self::Black => write!(f, "Noir"),
        }
    }
}

// ---------------------------------------------------------------------------

/// Type of a piece (independent of color).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceKind {
    /// FEN letter of the piece (lowercase).
    #[must_use]
    pub fn fen_char(self) -> char {
        match self {
            Self::Pawn   => 'p',
            Self::Knight => 'n',
            Self::Bishop => 'b',
            Self::Rook   => 'r',
            Self::Queen  => 'q',
            Self::King   => 'k',
        }
    }

    /// Builds a `PieceKind` from a FEN letter (case-insensitive).
    ///
    /// Returns `None` if the character is unknown.
    #[must_use]
    pub fn from_fen_char(c: char) -> Option<Self> {
        match c.to_ascii_lowercase() {
            'p' => Some(Self::Pawn),
            'n' => Some(Self::Knight),
            'b' => Some(Self::Bishop),
            'r' => Some(Self::Rook),
            'q' => Some(Self::Queen),
            'k' => Some(Self::King),
            _   => None,
        }
    }
}

impl std::fmt::Display for PieceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Pawn   => "Pion",
            Self::Knight => "Cavalier",
            Self::Bishop => "Fou",
            Self::Rook   => "Tour",
            Self::Queen  => "Dame",
            Self::King   => "Roi",
        };
        write!(f, "{name}")
    }
}

// ---------------------------------------------------------------------------

/// A piece: color + type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind:  PieceKind,
}

impl Piece {
    /// Builds a piece.
    #[must_use]
    #[inline]
    pub fn new(color: Color, kind: PieceKind) -> Self {
        Self { color, kind }
    }

    /// FEN letter of the piece (uppercase = white, lowercase = black).
    #[must_use]
    pub fn fen_char(self) -> char {
        let c = self.kind.fen_char();
        match self.color {
            Color::White => c.to_ascii_uppercase(),
            Color::Black => c,
        }
    }

    /// Builds a piece from a FEN letter.
    ///
    /// Returns `None` if the character is unknown.
    #[must_use]
    pub fn from_fen_char(c: char) -> Option<Self> {
        let color = if c.is_uppercase() { Color::White } else { Color::Black };
        let kind  = PieceKind::from_fen_char(c)?;
        Some(Self::new(color, kind))
    }
}

impl std::fmt::Display for Piece {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.color, self.kind)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_opposite() {
        assert_eq!(Color::White.opposite(), Color::Black);
        assert_eq!(Color::Black.opposite(), Color::White);
    }

    #[test]
    fn test_piece_fen_roundtrip() {
        let pieces = [
            Piece::new(Color::White, PieceKind::King),
            Piece::new(Color::Black, PieceKind::Queen),
            Piece::new(Color::White, PieceKind::Pawn),
            Piece::new(Color::Black, PieceKind::Knight),
        ];
        for piece in pieces {
            let c = piece.fen_char();
            let parsed = Piece::from_fen_char(c).unwrap();
            assert_eq!(piece, parsed);
        }
    }

    #[test]
    fn test_piece_fen_char_case() {
        assert!(Piece::new(Color::White, PieceKind::Rook).fen_char().is_uppercase());
        assert!(Piece::new(Color::Black, PieceKind::Rook).fen_char().is_lowercase());
    }

    #[test]
    fn test_piece_kind_from_fen_unknown() {
        assert!(PieceKind::from_fen_char('x').is_none());
    }
}
