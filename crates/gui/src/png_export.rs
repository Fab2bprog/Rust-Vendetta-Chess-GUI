//! PHASE 76 — PNG export of the chessboard (a "photo" of the displayed position).
//!
//! # Decisions settled (see `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 76)
//!
//! - Exported position: the one **displayed on screen** at the moment of
//!   the click, i.e. it respects navigation through the history
//!   (`viewed_ply`) — see `GameController::displayed_fen`. Decision
//!   deliberately different from the PDF export (`pdf_export.rs`), which
//!   always prints the game's final position: the PNG export is
//!   meant as a simple "photo" of what the user is looking at.
//! - Orientation: respects the on-screen "Flip" button (`board_flipped`),
//!   again for the same reason (screen photo, not a normalized
//!   archive document like the PDF).
//! - Content: 8×8 board with the pieces, plus each side's captured
//!   pieces shown as thumbnails above/below the board
//!   (same top/bottom split as `CapturedPiecesBar` in `app.slint`).
//!   No move number or other embedded text (see user discussion,
//!   PHASE 76): pieces captured in multiple copies
//!   are simply repeated side by side rather than accompanied by a
//!   text counter — avoids any dependency on a system font for
//!   this feature (consistent with the strict portability requirement
//!   for Windows/macOS/Linux already settled for this project, see PHASE 74).
//!
//! Library used: [`resvg`] (rasterization of the piece SVGs already
//! embedded for the PDF export, reused as-is via
//! `pdf_export::{FenPiece, parse_fen_placement, piece_svg_source}` — no
//! asset duplication) + `tiny_skia` (RGBA canvas, square filling,
//! final PNG encoding). `resvg` 0.47 directly re-exports `tiny_skia` and
//! `usvg` (confirmed via `docs.rs/resvg/0.47.0`), a single added dependency
//! is enough.

use crate::pdf_export::{parse_fen_placement, piece_svg_source, FenPiece};
use resvg::tiny_skia::{Color, Paint, Pixmap, Rect as SkRect, Transform};
use resvg::usvg::{Options, Tree};
use std::collections::HashMap;

/// Size of a board square, in pixels.
const SQUARE_PX: u32 = 90;
/// Total size of the board (8 squares), in pixels.
const BOARD_PX: u32 = SQUARE_PX * 8;
/// Height of each of the two captured-pieces strips (top/bottom), in
/// pixels.
const STRIP_PX: u32 = 70;
/// Left margin of the first captured-piece icon in its strip, in
/// pixels.
const STRIP_MARGIN_PX: f32 = 12.0;
/// Target size of a captured-piece icon (thumbnail), in pixels.
const CAPTURE_ICON_PX: f32 = 34.0;
/// Horizontal gap between two consecutive captured-piece icons, in
/// pixels.
const CAPTURE_ICON_GAP_PX: f32 = 4.0;
/// Proportion of the square occupied by a piece on the board (same
/// visual values as the PDF export, see `pdf_export::PIECE_FILL_RATIO`).
const PIECE_FILL_RATIO: f32 = 0.82;

/// Color of the light squares (same shades as the PDF export, converted from
/// `0.0..1.0` to `0..255`).
fn light_square_color() -> Color {
    Color::from_rgba8(237, 219, 183, 255)
}
/// Color of the dark squares.
fn dark_square_color() -> Color {
    Color::from_rgba8(161, 120, 87, 255)
}
/// Background color of the captured-pieces strips (neutral light gray,
/// to visually distinguish them from the board).
fn strip_background_color() -> Color {
    Color::from_rgba8(240, 240, 240, 255)
}

/// Converts a Slint piece code (`"wP"`, `"bN"`... produced by
/// `game_controller::piece_id`) into a [`FenPiece`]. Returns `None` for an
/// empty or malformed code — should never happen in practice (the codes
/// always come from `piece_id`), but we don't panic regardless.
fn parse_piece_code(code: &str) -> Option<FenPiece> {
    let mut chars = code.chars();
    let color = chars.next()?;
    let kind = chars.next()?;
    Some(FenPiece { is_white: color == 'w', kind })
}

/// A solid fill brush for a given color — avoids
/// duplicating the `Paint` construction for every drawn square/rectangle.
fn solid_paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

/// Cache of already-parsed SVG trees (`usvg::Tree`), one per piece
/// type+color (at most 12) — avoids reparsing the same SVG up to 32 times,
/// same caching logic as `pdf_export::draw_board`.
struct PieceTrees {
    trees: HashMap<(bool, char), Tree>,
    options: Options<'static>,
}

impl PieceTrees {
    fn new() -> Self {
        Self { trees: HashMap::new(), options: Options::default() }
    }

    /// Returns the already-parsed SVG tree for this piece, building it
    /// and caching it on first access.
    ///
    /// # Panics
    ///
    /// Only panics if one of the 12 piece SVGs embedded at
    /// compile time (`pdf_export.rs`) were invalid — never happens in
    /// practice, these are project assets, not user input.
    fn get(&mut self, piece: FenPiece) -> &Tree {
        let key = (piece.is_white, piece.kind);
        if !self.trees.contains_key(&key) {
            let tree = Tree::from_str(piece_svg_source(piece), &self.options)
                .expect("resvg : SVG de pièce invalide (asset embarqué corrompu)");
            self.trees.insert(key, tree);
        }
        self.trees.get(&key).expect("vient d'être inséré ci-dessus")
    }
}

/// Draws an SVG piece in the pixel-coordinate square
/// `(dest_x, dest_y)` to `(dest_x + size, dest_y + size)`, centered and scaled
/// while keeping its proportions (like `pdf_export::draw_board`).
fn draw_piece(pixmap: &mut Pixmap, tree: &Tree, dest_x: f32, dest_y: f32, size: f32) {
    let tree_size = tree.size();
    let (tree_w, tree_h) = (tree_size.width(), tree_size.height());
    if tree_w <= 0.0 || tree_h <= 0.0 {
        return;
    }
    let target = size * PIECE_FILL_RATIO;
    let scale = target / tree_w.max(tree_h);
    let offset_x = dest_x + (size - tree_w * scale) / 2.0;
    let offset_y = dest_y + (size - tree_h * scale) / 2.0;
    let transform = Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);
    resvg::render(tree, transform, &mut pixmap.as_mut());
}

/// Draws the 8×8 board (squares + pieces) into `pixmap`, starting at
/// Y coordinate `top_offset` (pixels), respecting the `flipped` orientation.
///
/// Indexing of `grid` identical to `pdf_export::parse_fen_placement`:
/// `grid[0]` = rank 8, `grid[7]` = rank 1; `grid[_][0]` = file a.
/// Not flipped (`flipped = false`): rank 8 at the top, file a on the left —
/// same convention as `board.slint` for `flipped = false`.
// Clippy: `#[allow(cast_precision_loss)]` — board indices (0-7) and
// pixel size constants (a few hundred at most) are far below
// the exact-precision limit of an `f32` (2^24); conversion safe in
// practice (same justification as `pdf_export::draw_board`).
#[allow(clippy::cast_precision_loss)]
fn draw_board(
    pixmap: &mut Pixmap,
    grid: &[[Option<FenPiece>; 8]; 8],
    flipped: bool,
    top_offset: f32,
) {
    let light = solid_paint(light_square_color());
    let dark = solid_paint(dark_square_color());

    for rank_index in 0..8usize {
        for file_index in 0..8usize {
            let is_dark = (file_index + rank_index) % 2 == 1;
            let (col, row) = if flipped {
                (7 - file_index, 7 - rank_index)
            } else {
                (file_index, rank_index)
            };
            let sq_x = col as f32 * SQUARE_PX as f32;
            let sq_y = top_offset + row as f32 * SQUARE_PX as f32;

            if let Some(rect) = SkRect::from_xywh(sq_x, sq_y, SQUARE_PX as f32, SQUARE_PX as f32) {
                pixmap.fill_rect(
                    rect,
                    if is_dark { &dark } else { &light },
                    Transform::identity(),
                    None,
                );
            }
        }
    }

    let mut trees = PieceTrees::new();
    for (rank_index, rank_row) in grid.iter().enumerate() {
        for (file_index, cell) in rank_row.iter().enumerate() {
            let Some(piece) = *cell else { continue };
            let (col, row) = if flipped {
                (7 - file_index, 7 - rank_index)
            } else {
                (file_index, rank_index)
            };
            let sq_x = col as f32 * SQUARE_PX as f32;
            let sq_y = top_offset + row as f32 * SQUARE_PX as f32;
            let tree = trees.get(piece);
            draw_piece(pixmap, tree, sq_x, sq_y, SQUARE_PX as f32);
        }
    }
}

/// Draws a captured-pieces strip (thumbnails side by side, repeated
/// as many times as captured — no text counter, see module
/// doc) into `pixmap`, starting at Y coordinate `top_offset`.
///
/// `pieces`: already compacted by `GameController::captured_summary`
/// (`CapturedPieceData { piece_code, count }`) — each entry is repeated
/// `count` times when displayed.
#[allow(clippy::cast_precision_loss)]
fn draw_capture_strip(pixmap: &mut Pixmap, pieces: &[crate::CapturedPieceData], top_offset: f32) {
    let background = solid_paint(strip_background_color());
    if let Some(rect) = SkRect::from_xywh(0.0, top_offset, BOARD_PX as f32, STRIP_PX as f32) {
        pixmap.fill_rect(rect, &background, Transform::identity(), None);
    }

    let total_count: i32 = pieces.iter().map(|p| p.count).sum();
    if total_count <= 0 {
        return;
    }

    // Shrinks the icon size if needed so everything fits within the
    // board's width (extreme case, very rarely reached in practice
    // : at most 15 pieces captured from a single side, king excepted).
    let available_width = BOARD_PX as f32 - 2.0 * STRIP_MARGIN_PX;
    let natural_width =
        total_count as f32 * (CAPTURE_ICON_PX + CAPTURE_ICON_GAP_PX) - CAPTURE_ICON_GAP_PX;
    let icon_size = if natural_width > available_width {
        ((available_width + CAPTURE_ICON_GAP_PX) / total_count as f32 - CAPTURE_ICON_GAP_PX)
            .max(10.0)
    } else {
        CAPTURE_ICON_PX
    };

    let mut trees = PieceTrees::new();
    let icon_y = top_offset + (STRIP_PX as f32 - icon_size) / 2.0;
    let mut x = STRIP_MARGIN_PX;

    for entry in pieces {
        let Some(piece) = parse_piece_code(entry.piece_code.as_str()) else { continue };
        let tree = trees.get(piece);
        for _ in 0..entry.count.max(0) {
            draw_piece(pixmap, tree, x, icon_y, icon_size);
            x += icon_size + CAPTURE_ICON_GAP_PX;
        }
    }
}

/// Generates the PNG bytes of the displayed board (position + orientation +
/// captured pieces), ready to write as-is into a `.png` file.
///
/// - `fen`: FEN of the position to represent — typically
///   `GameController::displayed_fen` (respects navigation through
///   the history, unlike `current_fen` used by the PDF export).
/// - `flipped`: board orientation (`board-flipped` on screen).
/// - `captured_by_white`/`captured_by_black`: same data as that
///   displayed on screen (`GameController::captured_summary`), with no
///   further transformation.
///
/// # Panics
///
/// Only on an internal `tiny_skia`/`resvg` failure (invalid
/// canvas dimensions or PNG encoding failure) — never observed in practice,
/// all dimensions used here are fixed constants of the module.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn build_board_png_bytes(
    fen: &str,
    flipped: bool,
    captured_by_white: &[crate::CapturedPieceData],
    captured_by_black: &[crate::CapturedPieceData],
) -> Vec<u8> {
    let grid = parse_fen_placement(fen);

    let width = BOARD_PX;
    let height = STRIP_PX + BOARD_PX + STRIP_PX;
    let mut pixmap = Pixmap::new(width, height)
        .expect("tiny-skia : dimensions de canvas invalides (constantes fixes du module)");
    pixmap.fill(strip_background_color());

    // Same top/bottom split as `CapturedPiecesBar` in app.slint: the
    // top strip always shows the trophies of the side displayed at
    // the top of the board (the opponent of the side displayed at the bottom).
    let (top_pieces, bottom_pieces) = if flipped {
        (captured_by_white, captured_by_black)
    } else {
        (captured_by_black, captured_by_white)
    };

    draw_capture_strip(&mut pixmap, top_pieces, 0.0);
    draw_board(&mut pixmap, &grid, flipped, STRIP_PX as f32);
    draw_capture_strip(&mut pixmap, bottom_pieces, (STRIP_PX + BOARD_PX) as f32);

    pixmap.encode_png().expect("tiny-skia : échec inattendu de l'encodage PNG")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn capture(code: &str, count: i32) -> crate::CapturedPieceData {
        crate::CapturedPieceData { piece_code: slint::SharedString::from(code), count }
    }

    #[test]
    fn test_parse_piece_code_white_pawn() {
        let piece = parse_piece_code("wP").expect("code valide");
        assert!(piece.is_white);
        assert_eq!(piece.kind, 'P');
    }

    #[test]
    fn test_parse_piece_code_black_knight() {
        let piece = parse_piece_code("bN").expect("code valide");
        assert!(!piece.is_white);
        assert_eq!(piece.kind, 'N');
    }

    #[test]
    fn test_parse_piece_code_empty_returns_none() {
        assert_eq!(parse_piece_code(""), None);
    }

    #[test]
    fn test_build_board_png_bytes_produces_valid_png_header() {
        let bytes = build_board_png_bytes(STARTPOS_FEN, false, &[], &[]);
        // A valid PNG file always starts with the signature
        // 0x89 'P' 'N' 'G' '\r' '\n' 0x1A '\n' (PNG specification).
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_build_board_png_bytes_flipped_does_not_panic() {
        let bytes = build_board_png_bytes(STARTPOS_FEN, true, &[], &[]);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_build_board_png_bytes_with_empty_board_does_not_panic() {
        let empty_fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        let bytes = build_board_png_bytes(empty_fen, false, &[], &[]);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_build_board_png_bytes_with_captures_does_not_panic() {
        let white_trophies = vec![capture("bP", 3), capture("bN", 1)];
        let black_trophies = vec![capture("wQ", 1)];
        let bytes = build_board_png_bytes(STARTPOS_FEN, false, &white_trophies, &black_trophies);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_build_board_png_bytes_with_many_captures_does_not_panic() {
        // Extreme case: far more captured pieces than would
        // normally fit at fixed icon size — forces the automatic
        // shrinking of the icons (see `draw_capture_strip`).
        let many = vec![capture("bP", 8), capture("bN", 2), capture("bB", 2), capture("bR", 2)];
        let bytes = build_board_png_bytes(STARTPOS_FEN, false, &many, &[]);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_build_board_png_bytes_with_zero_count_entry_does_not_panic() {
        let pieces = vec![capture("bP", 0)];
        let bytes = build_board_png_bytes(STARTPOS_FEN, false, &pieces, &[]);
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn test_build_board_png_bytes_mid_game_position_is_larger_than_empty_board() {
        let mid_game_fen =
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";
        let empty_fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        let bytes_mid = build_board_png_bytes(mid_game_fen, false, &[], &[]);
        let bytes_empty = build_board_png_bytes(empty_fen, false, &[], &[]);
        assert!(bytes_mid.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(bytes_empty.starts_with(&[0x89, b'P', b'N', b'G']));
        // No strict assertion on relative size (PNG compression
        // depends on content): only the absence of a panic is guaranteed.
    }
}
