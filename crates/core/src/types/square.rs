/// A square on the chessboard, encoded as an integer 0..=63.
///
/// Convention: `a1 = 0`, `b1 = 1`, …, `h8 = 63`.
/// File = index % 8, rank = index / 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Square(u8);

impl Square {
    /// Builds a square from a file (0-7) and a rank (0-7).
    ///
    /// # Panics
    /// Panics if `file > 7` or `rank > 7`.
    #[must_use]
    pub fn new(file: u8, rank: u8) -> Self {
        assert!(file < 8, "file doit être entre 0 et 7, reçu {file}");
        assert!(rank < 8, "rank doit être entre 0 et 7, reçu {rank}");
        Self(rank * 8 + file)
    }

    /// Builds a square from its raw index (0-63).
    ///
    /// # Panics
    /// Panics if `index > 63`.
    #[must_use]
    pub fn from_index(index: u8) -> Self {
        assert!(index < 64, "index doit être entre 0 et 63, reçu {index}");
        Self(index)
    }

    /// Parses algebraic notation (`"a1"` … `"h8"`).
    ///
    /// # Errors
    /// Returns `None` if the string is invalid.
    ///
    /// Clippy (04/07/2026): `#[allow(cast_possible_truncation)]` on the
    /// `u32 → u8` cast of the rank digit — `to_digit(10)` always returns a
    /// `0..=9` value, no truncation is possible in practice.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn from_algebraic(s: &str) -> Option<Self> {
        let mut chars = s.chars();
        let file_char = chars.next()?;
        let rank_char = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let file = (file_char.to_ascii_lowercase() as u8).checked_sub(b'a')?;
        let rank = (rank_char.to_digit(10)? as u8).checked_sub(1)?;
        if file > 7 || rank > 7 {
            return None;
        }
        Some(Self::new(file, rank))
    }

    /// Returns the raw index of the square (0-63).
    #[must_use]
    #[inline]
    pub fn index(self) -> u8 {
        self.0
    }

    /// File of the square (0 = a, 7 = h).
    #[must_use]
    #[inline]
    pub fn file(self) -> u8 {
        self.0 % 8
    }

    /// Rank of the square (0 = rank 1, 7 = rank 8).
    #[must_use]
    #[inline]
    pub fn rank(self) -> u8 {
        self.0 / 8
    }

    /// Algebraic notation of the square (`"a1"` … `"h8"`).
    #[must_use]
    pub fn to_algebraic(self) -> String {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        format!("{file}{rank}")
    }
}

impl std::fmt::Display for Square {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_algebraic())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_index() {
        let a1 = Square::new(0, 0);
        assert_eq!(a1.index(), 0);

        let h8 = Square::new(7, 7);
        assert_eq!(h8.index(), 63);

        let e4 = Square::new(4, 3);
        assert_eq!(e4.file(), 4);
        assert_eq!(e4.rank(), 3);
    }

    #[test]
    fn test_algebraic_roundtrip() {
        for index in 0u8..64 {
            let sq = Square::from_index(index);
            let alg = sq.to_algebraic();
            let parsed = Square::from_algebraic(&alg).unwrap();
            assert_eq!(sq, parsed, "Roundtrip échoué pour index {index}");
        }
    }

    #[test]
    fn test_from_algebraic_known_squares() {
        assert_eq!(Square::from_algebraic("a1").unwrap().index(), 0);
        assert_eq!(Square::from_algebraic("h1").unwrap().index(), 7);
        assert_eq!(Square::from_algebraic("a8").unwrap().index(), 56);
        assert_eq!(Square::from_algebraic("h8").unwrap().index(), 63);
        assert_eq!(Square::from_algebraic("e4").unwrap().index(), 28);
    }

    #[test]
    fn test_from_algebraic_invalid() {
        assert!(Square::from_algebraic("").is_none());
        assert!(Square::from_algebraic("a9").is_none());
        assert!(Square::from_algebraic("i1").is_none());
        assert!(Square::from_algebraic("a1b").is_none());
    }

    #[test]
    fn test_display() {
        assert_eq!(Square::new(4, 3).to_string(), "e4");
        assert_eq!(Square::new(0, 0).to_string(), "a1");
    }
}
