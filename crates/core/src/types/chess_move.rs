use super::{piece::PieceKind, square::Square};

/// Category of a move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoveKind {
    /// Normal move (simple move or capture).
    Normal,
    /// Castling (kingside or queenside).
    Castle,
    /// En passant capture.
    EnPassant,
    /// Pawn promotion.
    Promotion,
}

/// A chess move: starting square, destination square, type, optional promotion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move {
    /// Starting square.
    pub from: Square,
    /// Destination square.
    pub to: Square,
    /// Type of the move.
    pub kind: MoveKind,
    /// Promotion piece (only if `kind == MoveKind::Promotion`).
    pub promotion: Option<PieceKind>,
}

impl Move {
    /// Normal move (simple move or capture).
    #[must_use]
    #[inline]
    pub fn normal(from: Square, to: Square) -> Self {
        Self { from, to, kind: MoveKind::Normal, promotion: None }
    }

    /// Castling.
    #[must_use]
    #[inline]
    pub fn castle(from: Square, to: Square) -> Self {
        Self { from, to, kind: MoveKind::Castle, promotion: None }
    }

    /// En passant capture.
    #[must_use]
    #[inline]
    pub fn en_passant(from: Square, to: Square) -> Self {
        Self { from, to, kind: MoveKind::EnPassant, promotion: None }
    }

    /// Pawn promotion.
    ///
    /// # Panics
    /// Panics if `piece` is `PieceKind::Pawn` or `PieceKind::King`
    /// (illegal cases for promotion).
    #[must_use]
    pub fn promotion(from: Square, to: Square, piece: PieceKind) -> Self {
        assert!(
            !matches!(piece, PieceKind::Pawn | PieceKind::King),
            "Promotion invalide : impossible de promouvoir en Pion ou en Roi"
        );
        Self { from, to, kind: MoveKind::Promotion, promotion: Some(piece) }
    }

    /// UCI notation of the move (`"e2e4"`, `"e7e8q"` for a promotion).
    #[must_use]
    pub fn to_uci(self) -> String {
        let promo = self.promotion.map_or(String::new(), |p| p.fen_char().to_string());
        format!("{}{}{}", self.from, self.to, promo)
    }

    /// Parses a move from UCI notation (`"e2e4"`, `"e7e8q"`).
    ///
    /// Returns `None` if the string is invalid.
    ///
    /// Goes through `Vec<char>` rather than byte-slicing (`&s[0..2]`
    /// etc.): `s` can come from external/untrusted data (a corrupted
    /// or mis-encoded PGN import falling back to
    /// [`crate::pgn::resolve_san`]'s `from_uci` fallback path when no
    /// legal move matches by text). Byte-slicing on a non-ASCII string
    /// whose byte offsets do not land on a `char` boundary panics
    /// ("byte index N is not a char boundary"); iterating over `chars()`
    /// avoids that risk entirely, mirroring the same defensive pattern
    /// already used in `resolve_san`'s destination-square parsing.
    #[must_use]
    pub fn from_uci(s: &str) -> Option<Self> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < 4 || chars.len() > 5 {
            return None;
        }
        let from_str: String = chars[0..2].iter().collect();
        let to_str: String = chars[2..4].iter().collect();
        let from = Square::from_algebraic(&from_str)?;
        let to = Square::from_algebraic(&to_str)?;

        if chars.len() == 5 {
            let piece = PieceKind::from_fen_char(chars[4])?;
            Some(Self::promotion(from, to, piece))
        } else {
            Some(Self::normal(from, to))
        }
    }
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_move_uci() {
        let m = Move::normal(
            Square::from_algebraic("e2").unwrap(),
            Square::from_algebraic("e4").unwrap(),
        );
        assert_eq!(m.to_uci(), "e2e4");
    }

    #[test]
    fn test_promotion_uci() {
        let m = Move::promotion(
            Square::from_algebraic("e7").unwrap(),
            Square::from_algebraic("e8").unwrap(),
            PieceKind::Queen,
        );
        assert_eq!(m.to_uci(), "e7e8q");
    }

    #[test]
    fn test_from_uci_normal() {
        let m = Move::from_uci("e2e4").unwrap();
        assert_eq!(m.from, Square::from_algebraic("e2").unwrap());
        assert_eq!(m.to,   Square::from_algebraic("e4").unwrap());
        assert_eq!(m.kind, MoveKind::Normal);
        assert!(m.promotion.is_none());
    }

    #[test]
    fn test_from_uci_promotion() {
        let m = Move::from_uci("a7a8q").unwrap();
        assert_eq!(m.kind, MoveKind::Promotion);
        assert_eq!(m.promotion, Some(PieceKind::Queen));
    }

    #[test]
    fn test_from_uci_invalid() {
        assert!(Move::from_uci("").is_none());
        assert!(Move::from_uci("e2e").is_none());
        assert!(Move::from_uci("e2e4e5").is_none());
        assert!(Move::from_uci("z9z9").is_none());
    }

    #[test]
    fn test_uci_roundtrip() {
        for uci in ["e2e4", "d7d5", "e7e8q", "a7a8r"] {
            let m = Move::from_uci(uci).unwrap();
            assert_eq!(m.to_uci(), uci);
        }
    }

    #[test]
    #[should_panic(expected = "Promotion invalide")]
    fn test_promotion_pawn_panics() {
        let _ = Move::promotion(
            Square::from_algebraic("e7").unwrap(),
            Square::from_algebraic("e8").unwrap(),
            PieceKind::Pawn,
        );
    }
}
