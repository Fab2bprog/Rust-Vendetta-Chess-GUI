//! FEN serialization / deserialization for [`Position`].
//!
//! FEN (Forsyth–Edwards Notation) encodes a full position as a readable
//! string. Example of the starting position:
//! ```text
//! rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1
//! ```
//!
//! The 6 fields are: placement | side to move | castling | en passant | half-moves | move#.

use super::{
    board::Board,
    piece::Piece,
    position::{CastlingRights, Position},
    square::Square,
};

// ---------------------------------------------------------------------------
// FenError
// ---------------------------------------------------------------------------

/// Error produced while parsing a FEN string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenError {
    /// The number of fields is incorrect (6 expected).
    InvalidFieldCount(usize),
    /// The piece placement is invalid.
    InvalidPiecePlacement(String),
    /// The active color is invalid (`w` or `b` expected).
    InvalidActiveColor(String),
    /// The castling rights are invalid.
    InvalidCastlingRights(String),
    /// The en passant square is invalid.
    InvalidEnPassant(String),
    /// The half-move clock is invalid.
    InvalidHalfmoveClock(String),
    /// The full move number is invalid.
    InvalidFullmoveNumber(String),
    /// Invalid king count (exactly one king per color is required).
    InvalidKingCount { white: usize, black: usize },
}

impl std::fmt::Display for FenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFieldCount(n) =>
                write!(f, "FEN invalide : {n} champ(s) trouvé(s), 6 attendus"),
            Self::InvalidPiecePlacement(s) =>
                write!(f, "Placement de pièces invalide : {s}"),
            Self::InvalidActiveColor(s) =>
                write!(f, "Couleur active invalide : '{s}' (attendu 'w' ou 'b')"),
            Self::InvalidCastlingRights(s) =>
                write!(f, "Droits de roque invalides : '{s}'"),
            Self::InvalidEnPassant(s) =>
                write!(f, "Case en passant invalide : '{s}'"),
            Self::InvalidHalfmoveClock(s) =>
                write!(f, "Horloge demi-coups invalide : '{s}'"),
            Self::InvalidFullmoveNumber(s) =>
                write!(f, "Numéro de coup invalide : '{s}'"),
            Self::InvalidKingCount { white, black } =>
                write!(f, "Nombre de rois invalide : {white} blanc(s), {black} noir(s) (1 attendu par couleur)"),
        }
    }
}

impl std::error::Error for FenError {}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parses the piece placement field (first FEN field).
fn parse_board(placement: &str) -> Result<Board, FenError> {
    let ranks: Vec<&str> = placement.split('/').collect();

    if ranks.len() != 8 {
        return Err(FenError::InvalidPiecePlacement(format!(
            "8 rangées attendues, {} trouvée(s)",
            ranks.len()
        )));
    }

    let mut board = Board::empty();

    for (rank_idx, rank_str) in ranks.iter().enumerate() {
        // FEN starts with rank 8 (index 7 in our representation)
        #[allow(clippy::cast_possible_truncation)]
        let rank = 7u8 - rank_idx as u8;
        let mut file: u8 = 0;

        for ch in rank_str.chars() {
            if ch.is_ascii_digit() && ch != '0' {
                let skip = ch as u8 - b'0';
                file = file.saturating_add(skip);
            } else {
                if file >= 8 {
                    return Err(FenError::InvalidPiecePlacement(format!(
                        "Rangée trop longue : '{rank_str}'"
                    )));
                }
                let piece = Piece::from_fen_char(ch).ok_or_else(|| {
                    FenError::InvalidPiecePlacement(format!(
                        "Caractère de pièce inconnu : '{ch}'"
                    ))
                })?;
                board.set_piece(Square::new(file, rank), Some(piece));
                file += 1;
            }
        }

        if file != 8 {
            return Err(FenError::InvalidPiecePlacement(format!(
                "Rangée '{rank_str}' couvre {file} case(s) au lieu de 8"
            )));
        }
    }

    Ok(board)
}

/// Generates the piece placement field from a `Board`.
fn board_to_fen_field(board: &Board) -> String {
    let mut result = String::with_capacity(64);

    for rank in (0u8..8).rev() {
        let mut empty: u8 = 0;

        for file in 0u8..8 {
            match board.piece_at(Square::new(file, rank)) {
                None => empty += 1,
                Some(piece) => {
                    if empty > 0 {
                        result.push(char::from(b'0' + empty));
                        empty = 0;
                    }
                    result.push(piece.fen_char());
                }
            }
        }

        if empty > 0 {
            result.push(char::from(b'0' + empty));
        }
        if rank > 0 {
            result.push('/');
        }
    }

    result
}

/// Parses the castling rights (third FEN field).
fn parse_castling(s: &str) -> Result<CastlingRights, FenError> {
    if s == "-" {
        return Ok(CastlingRights::none());
    }

    let mut rights = CastlingRights::none();

    for ch in s.chars() {
        match ch {
            'K' => rights.white_kingside  = true,
            'Q' => rights.white_queenside = true,
            'k' => rights.black_kingside  = true,
            'q' => rights.black_queenside = true,
            _   => return Err(FenError::InvalidCastlingRights(s.to_owned())),
        }
    }

    Ok(rights)
}

// ---------------------------------------------------------------------------
// impl Position
// ---------------------------------------------------------------------------

impl Position {
    /// Parses a position from a FEN string.
    ///
    /// # Errors
    /// Returns [`FenError`] if the string is malformed.
    ///
    /// # Example
    /// ```
    /// use core::types::Position;
    ///
    /// let pos = Position::from_fen(
    ///     "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
    /// ).unwrap();
    /// ```
    pub fn from_fen(fen: &str) -> Result<Self, FenError> {
        let fields: Vec<&str> = fen.split_whitespace().collect();

        if fields.len() != 6 {
            return Err(FenError::InvalidFieldCount(fields.len()));
        }

        // 1. Piece placement
        let board = parse_board(fields[0])?;

        // NOTE: the king count is NOT validated here. `Position` is a
        // low-level type reused by many internal tests for
        // partial positions (e.g. a single king to isolate a move
        // generation test) — enforcing the "one king per color" rule at this
        // level would break these legitimate uses. The "playable
        // position" validation (exactly one king per color) is applied further up,
        // at the boundary of an actual game: see `GameState::from_fen`
        // (crate::game), which is the entry point used for any FEN
        // provided by the user (paste FEN, wizard, position editor).

        // 2. Active color
        let side_to_move = match fields[1] {
            "w" => super::piece::Color::White,
            "b" => super::piece::Color::Black,
            s   => return Err(FenError::InvalidActiveColor(s.to_owned())),
        };

        // 3. Castling rights
        let castling = parse_castling(fields[2])?;

        // 4. En passant
        let en_passant = if fields[3] == "-" {
            None
        } else {
            Some(
                Square::from_algebraic(fields[3])
                    .ok_or_else(|| FenError::InvalidEnPassant(fields[3].to_owned()))?,
            )
        };

        // 5. Half-move clock
        let halfmove_clock = fields[4]
            .parse::<u8>()
            .map_err(|_| FenError::InvalidHalfmoveClock(fields[4].to_owned()))?;

        // 6. Full move number
        let fullmove_number = fields[5]
            .parse::<u16>()
            .map_err(|_| FenError::InvalidFullmoveNumber(fields[5].to_owned()))?;

        Ok(Self {
            board,
            side_to_move,
            castling,
            en_passant,
            halfmove_clock,
            fullmove_number,
        })
    }

    /// Generates the FEN string of the position.
    #[must_use]
    pub fn to_fen(&self) -> String {
        let placement = board_to_fen_field(&self.board);

        let color = match self.side_to_move {
            super::piece::Color::White => "w",
            super::piece::Color::Black => "b",
        };

        let castling = self.castling.to_fen();

        let en_passant = self
            .en_passant
            .map_or_else(|| "-".to_owned(), Square::to_algebraic);

        format!(
            "{placement} {color} {castling} {en_passant} {} {}",
            self.halfmove_clock, self.fullmove_number
        )
    }

    /// Position identity key for threefold repetition detection.
    ///
    /// Two positions are considered identical for this rule if they
    /// have the same piece placement, the same side to move, the same
    /// remaining castling rights, and the same en passant square (if set). The
    /// half-move / full move counters (which change on every move and
    /// have no bearing on position identity) are excluded.
    ///
    /// Pragmatic simplification: the en passant square is taken into
    /// account as soon as it is present in the FEN, without checking that an
    /// en passant capture is actually playable there — a very rare edge
    /// case, ignored by most mainstream GUIs/engines.
    #[must_use]
    pub fn repetition_key(&self) -> String {
        let placement = board_to_fen_field(&self.board);
        let color = match self.side_to_move {
            super::piece::Color::White => "w",
            super::piece::Color::Black => "b",
        };
        let castling = self.castling.to_fen();
        let en_passant = self
            .en_passant
            .map_or_else(|| "-".to_owned(), Square::to_algebraic);
        format!("{placement} {color} {castling} {en_passant}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::piece::{Color, PieceKind, Piece};

    const START_FEN: &str =
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    #[test]
    fn test_parse_starting_fen() {
        let pos = Position::from_fen(START_FEN).unwrap();
        assert_eq!(pos.side_to_move, Color::White);
        assert_eq!(pos.halfmove_clock, 0);
        assert_eq!(pos.fullmove_number, 1);
        assert!(pos.castling.white_kingside);
        assert!(pos.castling.black_queenside);
        assert!(pos.en_passant.is_none());
    }

    #[test]
    fn test_starting_fen_roundtrip() {
        let pos = Position::from_fen(START_FEN).unwrap();
        assert_eq!(pos.to_fen(), START_FEN);
    }

    #[test]
    fn test_starting_fen_matches_starting_position() {
        let from_fen   = Position::from_fen(START_FEN).unwrap();
        let from_start = Position::starting();
        // Check a few key pieces
        let e1 = Square::from_algebraic("e1").unwrap();
        let e8 = Square::from_algebraic("e8").unwrap();
        assert_eq!(from_fen.piece_at(e1), from_start.piece_at(e1));
        assert_eq!(from_fen.piece_at(e8), from_start.piece_at(e8));
        assert_eq!(from_fen.board.total_pieces(), 32);
    }

    #[test]
    fn test_parse_position_after_e4() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(pos.side_to_move, Color::Black);
        assert_eq!(
            pos.en_passant,
            Some(Square::from_algebraic("e3").unwrap())
        );
        // The white pawn is on e4
        let e4 = Square::from_algebraic("e4").unwrap();
        assert_eq!(pos.piece_at(e4), Some(Piece::new(Color::White, PieceKind::Pawn)));
        // e2 is empty
        let e2 = Square::from_algebraic("e2").unwrap();
        assert!(pos.piece_at(e2).is_none());
    }

    #[test]
    fn test_fen_roundtrip_after_e4() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(pos.to_fen(), fen);
    }

    #[test]
    fn test_fen_no_castling_rights() {
        let fen = "8/8/8/8/8/8/8/4K2k w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        assert!(!pos.castling.white_kingside);
        assert!(!pos.castling.white_queenside);
        assert!(!pos.castling.black_kingside);
        assert!(!pos.castling.black_queenside);
        assert_eq!(pos.to_fen(), fen);
    }

    #[test]
    fn test_fen_halfmove_and_fullmove() {
        let fen = "8/8/8/8/8/8/8/4K2k b - - 42 100";
        let pos = Position::from_fen(fen).unwrap();
        assert_eq!(pos.halfmove_clock, 42);
        assert_eq!(pos.fullmove_number, 100);
        assert_eq!(pos.to_fen(), fen);
    }

    #[test]
    fn test_fen_error_wrong_field_count() {
        assert!(matches!(
            Position::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -"),
            Err(FenError::InvalidFieldCount(4))
        ));
    }

    #[test]
    fn test_fen_error_invalid_color() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(fen),
            Err(FenError::InvalidActiveColor(_))
        ));
    }

    #[test]
    fn test_fen_error_invalid_castling() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w XYZ - 0 1";
        assert!(matches!(
            Position::from_fen(fen),
            Err(FenError::InvalidCastlingRights(_))
        ));
    }

    #[test]
    fn test_fen_error_invalid_en_passant() {
        let fen = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq z9 0 1";
        assert!(matches!(
            Position::from_fen(fen),
            Err(FenError::InvalidEnPassant(_))
        ));
    }

    #[test]
    fn test_fen_error_invalid_piece() {
        let fen = "rnbqkXnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(fen),
            Err(FenError::InvalidPiecePlacement(_))
        ));
    }

    #[test]
    fn test_fen_error_rank_wrong_length() {
        // Rank with 9 squares
        let fen = "rnbqkbnr1/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert!(matches!(
            Position::from_fen(fen),
            Err(FenError::InvalidPiecePlacement(_))
        ));
    }
}
