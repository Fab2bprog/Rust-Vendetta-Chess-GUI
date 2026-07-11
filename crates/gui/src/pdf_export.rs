//! PHASE 25 — PDF printing of the game.
//!
//! Generates a PDF document containing the board (final position) and the
//! move list of a game, regardless of its type (Human vs Human,
//! Human vs Engine, Engine vs Engine, tournament, puzzle...).
//!
//! # Decisions settled (see `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 25)
//!
//! - Printed position: always the game's **final position**
//!   (last position played), regardless of the position viewed on
//!   screen at the moment "Print" was clicked (navigation through the history
//!   has no effect on printing).
//! - Move list: full-grid table, columns No. | White |
//!   Black | Comment (empty column meant for handwritten annotation
//!   on paper — user request, 08/07/2026), same data source
//!   as the move panel shown on screen (`GameController::build_move_rows`,
//!   no duplicated logic).
//! - Header banner: players/engines, date, time control, and result
//!   only if the game is finished.
//! - Board orientation: always White at the bottom, regardless of
//!   the on-screen "Flip" button state.
//!
//! Library used: [`printpdf`] (vector drawing of the squares + the
//! table, and direct embedding of the piece SVGs already present in
//! `crates/gui/assets/pieces/*.svg` via the crate's `svg` feature —
//! no rasterization or asset duplication needed).
//!
//! # Progress
//!
//! - Step 1: input data structures + blank A4 page.
//! - Step 2: drawing of the header banner (title, players/engines, date,
//!   time control, result if present).
//! - Step 3: drawing of the board (8×8 grid, pieces of the
//!   final position via the SVGs already used on screen).
//! - Step 4: drawing of the moves table (two columns, automatic
//!   pagination across several pages if needed).
//! - Step 5: wiring of the "Print" button in `app.slint`/`main.rs`, and
//!   [`today_date_string`] (today's date with no new dependency) to
//!   populate [`PrintGameInfo::date`].
//! - Step 6: end-to-end integration tests (header + board +
//!   moves table combined into a single document) — PHASE 25 complete.
//!
//! Follow-up note (visual rendering, to check once the feature is wired
//! end-to-end into the main screen): the standard PDF fonts (14
//! base fonts, `WinAnsi` encoding) used here cover the common
//! accented French characters, but have not been visually checked on
//! a real PDF reader at this stage — only non-regression of the generated file
//! (`%PDF-` header, absence of panic) is validated by `cargo test`.
//!
//! Follow-up note (Step 3, `PaintMode` choice): the board squares are
//! drawn with `PaintMode::Fill` (fill only, no outline).
//!
//! Fix from 03/07/2026 (first `cargo test`): `PaintMode` and
//! `WindingOrder` are **not** re-exported at the root of the `printpdf` crate
//! 0.7.0 (compile error `E0432: no 'PaintMode'/'WindingOrder' in the
//! root`) — they live in the `printpdf::path` submodule
//! (confirmed via `https://docs.rs/printpdf/0.7.0/printpdf/path/enum.PaintMode.html`
//! and `.../enum.WindingOrder.html`, which also show that `PaintMode::Fill`
//! does indeed exist, contrary to what had been cautiously assumed
//! when this step was first written).

use printpdf::path::{PaintMode, WindingOrder};
use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point, Rect,
    Rgb, Svg, SvgTransform, SvgXObjectRef,
};
use std::collections::HashMap;

/// Width of an A4 page, in millimeters.
pub(crate) const PAGE_WIDTH_MM: f32 = 210.0;
/// Height of an A4 page, in millimeters.
pub(crate) const PAGE_HEIGHT_MM: f32 = 297.0;

/// Left/top margin used for every element drawn on the page.
const MARGIN_MM: f32 = 15.0;
/// Font size of the title ("Vendetta Chess — Partie").
const TITLE_FONT_SIZE: f32 = 16.0;
/// Font size of the header banner lines (players, date, time control,
/// result).
const HEADER_FONT_SIZE: f32 = 11.0;
/// Height of a header banner line, in millimeters.
const HEADER_LINE_HEIGHT_MM: f32 = 7.0;

/// Total size of the board (8 squares), in millimeters.
// Reduced from 160 mm to 40 mm on 08/07/2026 (board deemed too large), then
// raised back to 60 mm the same day (40 mm deemed too small after trying it).
const BOARD_SIZE_MM: f32 = 60.0;
/// Size of a square, in millimeters.
const SQUARE_SIZE_MM: f32 = BOARD_SIZE_MM / 8.0;
/// Vertical gap left above the board (below the header banner)
/// and below it (before the moves table, Step 4).
const BOARD_GAP_MM: f32 = 8.0;
/// Proportion of the square occupied by a piece (visual margin to avoid
/// a piece touching the edges of its square).
const PIECE_FILL_RATIO: f32 = 0.82;

/// Color of the light squares (RGB, 0.0–1.0 components).
const LIGHT_SQUARE_RGB: (f32, f32, f32) = (0.93, 0.86, 0.72);
/// Color of the dark squares (RGB, 0.0–1.0 components).
const DARK_SQUARE_RGB: (f32, f32, f32) = (0.63, 0.47, 0.34);

/// Font size of the moves table (column header and rows).
const MOVES_FONT_SIZE: f32 = 10.0;
/// Height of a row in the moves table (header or move), in millimeters.
const MOVES_ROW_HEIGHT_MM: f32 = 6.0;
/// Width of the "No." column of the moves table, in millimeters.
// Narrowed on 08/07/2026 (18 → 12 mm, per user request): amply
// sufficient for a move number ("42.").
const MOVES_NUMBER_COL_WIDTH_MM: f32 = 12.0;
/// Width of each of the "White"/"Black" columns of the moves table.
// Narrowed on 08/07/2026: fixed value (instead of sharing all the
// remaining page width, ~81 mm, far too wide for a move in
// SAN notation) — 24 mm remains comfortable even for "Dxd5+" or "O-O-O".
const MOVES_MOVE_COL_WIDTH_MM: f32 = 24.0;
/// Width of the "Comment" column of the moves table (08/07/2026,
/// user request) — left empty when printed, meant for
/// handwritten annotation on paper. Occupies all of the remaining page width
/// after the three previous columns.
const MOVES_COMMENT_COL_WIDTH_MM: f32 = PAGE_WIDTH_MM
    - 2.0 * MARGIN_MM
    - MOVES_NUMBER_COL_WIDTH_MM
    - 2.0 * MOVES_MOVE_COL_WIDTH_MM;
/// Left inner margin of the text in each cell (between the
/// grid line and the start of the text), in millimeters.
const MOVES_CELL_TEXT_PADDING_MM: f32 = 1.5;
/// Space left between the top grid line of a row and the baseline
/// of the text it contains, in millimeters — prevents the text from
/// touching the line above. Chosen so that the bottom line of a
/// row coincides exactly with the top line of the next one (no
/// gap or overlap, see `draw_moves_table`).
const MOVES_ROW_TOP_PADDING_MM: f32 = 1.6;

/// A chess piece as decoded from a FEN placement character.
///
/// `pub(crate)` (PHASE 76): reused as-is by `png_export.rs` to
/// avoid duplicating the FEN decoding / the 12 embedded piece SVGs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FenPiece {
    /// `true` for a white piece (uppercase FEN letter), `false` for
    /// a black piece (lowercase FEN letter).
    pub(crate) is_white: bool,
    /// Piece type, always uppercase ('K', 'Q', 'R', 'B', 'N', 'P').
    pub(crate) kind: char,
}

/// Decodes the "piece placement" field of a FEN string (first part,
/// before the first space) into a `[rank][file]` grid.
///
/// Indexing: `grid[0]` corresponds to the first segment delimited by `/`
/// in the FEN, i.e. rank 8 (top of the board as seen with White at the
/// bottom); `grid[7]` corresponds to rank 1 (bottom of the board). This
/// indexing directly matches FEN's natural order, with no need
/// for flip logic: PHASE 25 imposes a fixed orientation with
/// White at the bottom (see decisions settled at the top of the module).
/// `grid[rank][file]`, `file` 0 = file a … 7 = file h.
///
/// Tolerant to a malformed or truncated FEN: never panics, simply
/// fills the remaining squares with `None` (no piece).
pub(crate) fn parse_fen_placement(fen: &str) -> [[Option<FenPiece>; 8]; 8] {
    let mut grid: [[Option<FenPiece>; 8]; 8] = [[None; 8]; 8];

    let placement = fen.split_whitespace().next().unwrap_or("");

    for (rank_index, rank_str) in placement.split('/').enumerate() {
        if rank_index >= 8 {
            break;
        }
        let mut file_index = 0usize;
        for c in rank_str.chars() {
            if file_index >= 8 {
                break;
            }
            if let Some(empty_count) = c.to_digit(10) {
                file_index += empty_count as usize;
                continue;
            }
            let kind = c.to_ascii_uppercase();
            if matches!(kind, 'K' | 'Q' | 'R' | 'B' | 'N' | 'P') {
                grid[rank_index][file_index] = Some(FenPiece {
                    is_white: c.is_ascii_uppercase(),
                    kind,
                });
            }
            file_index += 1;
        }
    }

    grid
}

// The 12 piece SVGs already used by the on-screen board
// (`crates/gui/assets/pieces/*.svg`) — embedded directly at
// compile time, with no duplication or rasterization (decision settled with
// the user, see SUIVI_PLAN_ACTION.md, PHASE 25).
const SVG_WK: &str = include_str!("../assets/pieces/wK.svg");
const SVG_WQ: &str = include_str!("../assets/pieces/wQ.svg");
const SVG_WR: &str = include_str!("../assets/pieces/wR.svg");
const SVG_WB: &str = include_str!("../assets/pieces/wB.svg");
const SVG_WN: &str = include_str!("../assets/pieces/wN.svg");
const SVG_WP: &str = include_str!("../assets/pieces/wP.svg");
const SVG_BK: &str = include_str!("../assets/pieces/bK.svg");
const SVG_BQ: &str = include_str!("../assets/pieces/bQ.svg");
const SVG_BR: &str = include_str!("../assets/pieces/bR.svg");
const SVG_BB: &str = include_str!("../assets/pieces/bB.svg");
const SVG_BN: &str = include_str!("../assets/pieces/bN.svg");
const SVG_BP: &str = include_str!("../assets/pieces/bP.svg");

/// Returns the source SVG content corresponding to a [`FenPiece`].
///
/// Unrecognized piece/color (should not happen, `kind` is always
/// normalized by [`parse_fen_placement`]): returns the corresponding
/// pawn by default rather than panicking.
pub(crate) fn piece_svg_source(piece: FenPiece) -> &'static str {
    match (piece.is_white, piece.kind) {
        (true, 'K') => SVG_WK,
        (true, 'Q') => SVG_WQ,
        (true, 'R') => SVG_WR,
        (true, 'B') => SVG_WB,
        (true, 'N') => SVG_WN,
        (true, 'P') => SVG_WP,
        (false, 'K') => SVG_BK,
        (false, 'Q') => SVG_BQ,
        (false, 'R') => SVG_BR,
        (false, 'B') => SVG_BB,
        (false, 'N') => SVG_BN,
        (false, 'P') => SVG_BP,
        _ => if piece.is_white { SVG_WP } else { SVG_BP },
    }
}

/// SVG `XObject` of a piece already registered in the page, with the
/// physical dimensions (mm) and the DPI computed once for this piece
/// type (same values reused for each occurrence on
/// the board, potentially up to 8 times for a pawn).
struct PieceGraphic {
    xobject: SvgXObjectRef,
    dpi: f32,
    width_mm: f32,
    height_mm: f32,
}

/// Header banner printed at the top of the PDF (drawn starting from Step 2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrintGameInfo {
    /// Name of the White player/engine.
    pub white_name: String,
    /// Name of the Black player/engine.
    pub black_name: String,
    /// Date of the game, already formatted for display.
    pub date: String,
    /// Time control label, already formatted for display (e.g. "5+0 min",
    /// "Unlimited").
    pub time_control_label: String,
    /// Game result ("1-0", "0-1", "1/2-1/2"...). `None` as long as the
    /// game is not finished — in that case no result line is
    /// printed (decision settled for PHASE 25, no misleading "in
    /// progress" on a document meant to be archived).
    pub result: Option<String>,
}

/// Today's date, formatted `DD/MM/YYYY` — meant to populate
/// [`PrintGameInfo::date`] when "Print" is clicked (Step 5).
///
/// No date/time dependency added on purpose for this simple timestamp:
/// purely arithmetic computation from the system clock
/// (`std::time::SystemTime`) via Howard Hinnant's public
/// `civil_from_days` algorithm (public domain, see
/// <http://howardhinnant.github.io/date_algorithms.html>), already used by
/// many reference libraries (libc++, abseil...). Consistent
/// with PHASE 25's choice to add only a single external dependency
/// (`printpdf`) for the entire printing feature.
// Clippy (04/07/2026): `#[allow(cast_possible_wrap)]` — `as_secs()` (u64,
// seconds since 1970) comfortably fits in an `i64` before the year 292
// billion; conversion safe by construction for this use.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn today_date_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let days = secs.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{day:02}/{month:02}/{year:04}")
}

/// Converts a number of days elapsed since the Unix epoch (1970-01-01,
/// day 0) into a Gregorian civil date `(year, month, day)`. Pure function,
/// with no external dependency — see [`today_date_string`].
// Clippy: `#[allow(cast_sign_loss, cast_possible_wrap, cast_possible_truncation)]`
// — Howard Hinnant's algorithm (public domain); the values handled
// stay far below the bounds of `u64`/`i64`/`u32` for any plausible
// civil date, the conversions are safe by construction of the algorithm
// (same justification as `debug_log::civil_from_days`, deliberate copy).
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Generates the bytes of a PDF document for the given game.
///
/// - `final_fen`: FEN of the game's final position — always the
///   last position played (see `GameController::current_fen`, which
///   reflects the game's real state regardless of navigation through
///   the on-screen history; the board's display separately follows
///   `viewed_ply`).
/// - `moves`: move list already formatted for display, same source as
///   the on-screen move panel (`GameController::build_move_rows`) — no
///   duplicated logic to rebuild the move list.
/// - `info`: header banner (players/engines, date, time control, result).
///
/// # Panics
///
/// Only panics on an internal failure (never observed in practice)
/// of PDF serialization by `printpdf` — not an expected error
/// condition under normal operation.
#[must_use]
pub fn build_pdf_bytes(
    final_fen: &str,
    moves: &[crate::MoveRow],
    info: &PrintGameInfo,
) -> Vec<u8> {
    let (doc, page1, layer1) = PdfDocument::new(
        "Vendetta Chess — Partie",
        Mm(PAGE_WIDTH_MM),
        Mm(PAGE_HEIGHT_MM),
        "Contenu",
    );
    let current_layer = doc.get_page(page1).get_layer(layer1);

    // Standard PDF fonts (14 base fonts, always available with no
    // font file to embed): bold title, rest of the banner in
    // normal font.
    let title_font = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .expect("printpdf : police standard Helvetica-Bold introuvable");
    let body_font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .expect("printpdf : police standard Helvetica introuvable");

    let header_bottom_y = draw_header(&current_layer, &title_font, &body_font, info);
    let board_bottom_y = draw_board(&current_layer, header_bottom_y, final_fen);
    draw_moves_table(&doc, page1, layer1, board_bottom_y, &title_font, &body_font, moves);

    doc.save_to_bytes()
        .expect("printpdf : échec inattendu de la sérialisation du PDF")
}

/// Draws the header banner (title, players/engines, date, time control,
/// result if present) at the top of the page, starting `MARGIN_MM` from the
/// left edge and the top of the page.
///
/// Returns the Y coordinate (in millimeters, origin at the bottom of the page as
/// in the entire `printpdf` coordinate system) right below the last
/// line drawn — the following steps (board, moves table)
/// can use it to avoid overlapping the banner.
fn draw_header(
    layer: &PdfLayerReference,
    title_font: &IndirectFontRef,
    body_font: &IndirectFontRef,
    info: &PrintGameInfo,
) -> f32 {
    let mut y = PAGE_HEIGHT_MM - MARGIN_MM;

    layer.use_text(
        "Vendetta Chess — Partie",
        TITLE_FONT_SIZE,
        Mm(MARGIN_MM),
        Mm(y),
        title_font,
    );
    y -= HEADER_LINE_HEIGHT_MM * 1.6;

    let players_line = format!("Blancs : {}    Noirs : {}", info.white_name, info.black_name);
    layer.use_text(players_line.as_str(), HEADER_FONT_SIZE, Mm(MARGIN_MM), Mm(y), body_font);
    y -= HEADER_LINE_HEIGHT_MM;

    let meta_line = format!(
        "Date : {}    Cadence : {}",
        info.date, info.time_control_label
    );
    layer.use_text(meta_line.as_str(), HEADER_FONT_SIZE, Mm(MARGIN_MM), Mm(y), body_font);
    y -= HEADER_LINE_HEIGHT_MM;

    if let Some(result) = &info.result {
        let result_line = format!("Résultat : {result}");
        layer.use_text(result_line.as_str(), HEADER_FONT_SIZE, Mm(MARGIN_MM), Mm(y), body_font);
        y -= HEADER_LINE_HEIGHT_MM;
    }

    y
}

/// Draws the board (8×8 grid + pieces of the `final_fen` position) below
/// the Y coordinate `top_y` (typically the value returned by
/// [`draw_header`]), aligned on the page's left margin.
///
/// Orientation always White at the bottom (decision settled for PHASE 25,
/// independent of the on-screen "Flip" button): no flip logic
/// is needed, [`parse_fen_placement`] already indexes the grid in this
/// order.
///
/// Returns the Y coordinate right below the board (with the
/// [`BOARD_GAP_MM`] margin already deducted), for Step 4 (moves table).
// Clippy: `#[allow(cast_precision_loss)]` — board indices (0-7) and
// SVG dimensions in pixels (a few hundred at most) are far below the
// exact-precision limit of an `f32` (2^24); conversion safe in practice.
#[allow(clippy::cast_precision_loss)]
fn draw_board(layer: &PdfLayerReference, top_y: f32, final_fen: &str) -> f32 {
    let board_top = top_y - BOARD_GAP_MM;
    let board_left = MARGIN_MM;
    let grid = parse_fen_placement(final_fen);

    // 1. Squares: drawn first to stay behind the pieces.
    //    `PaintMode::Fill`: fill only, no outline.
    for rank_index in 0..8usize {
        for file_index in 0..8usize {
            let rank_0idx = 7 - rank_index;
            let is_dark = (file_index + rank_0idx).is_multiple_of(2);
            let (r, g, b) = if is_dark { DARK_SQUARE_RGB } else { LIGHT_SQUARE_RGB };

            let sq_left = board_left + file_index as f32 * SQUARE_SIZE_MM;
            let sq_bottom = board_top - (rank_index as f32 + 1.0) * SQUARE_SIZE_MM;
            let sq_right = sq_left + SQUARE_SIZE_MM;
            let sq_top = sq_bottom + SQUARE_SIZE_MM;

            layer.set_fill_color(Color::Rgb(Rgb::new(r, g, b, None)));
            let rect = Rect::new(Mm(sq_left), Mm(sq_bottom), Mm(sq_right), Mm(sq_top))
                .with_mode(PaintMode::Fill)
                .with_winding(WindingOrder::NonZero);
            layer.add_rect(rect);
        }
    }

    // 2. Pieces: one SVG XObject per piece type+color (at most 12),
    //    created once then reused (cloned) for each occurrence
    //    on the board — avoids reparsing the same SVG up to 32 times.
    let mut graphics: HashMap<(bool, char), PieceGraphic> = HashMap::new();

    for (rank_index, rank_row) in grid.iter().enumerate() {
        for (file_index, cell) in rank_row.iter().enumerate() {
            let Some(piece) = *cell else {
                continue;
            };

            let graphic = graphics.entry((piece.is_white, piece.kind)).or_insert_with(|| {
                let svg = Svg::parse(piece_svg_source(piece))
                    .expect("printpdf : SVG de pièce invalide (asset embarqué corrompu)");
                let width_px = svg.width.0 as f32;
                let height_px = svg.height.0 as f32;
                let max_px = width_px.max(height_px);
                let target_mm = SQUARE_SIZE_MM * PIECE_FILL_RATIO;
                let dpi = if max_px > 0.0 {
                    max_px * 25.4 / target_mm
                } else {
                    300.0
                };
                let width_mm = width_px * 25.4 / dpi;
                let height_mm = height_px * 25.4 / dpi;
                let xobject = svg.into_xobject(layer);
                PieceGraphic { xobject, dpi, width_mm, height_mm }
            });

            let sq_left = board_left + file_index as f32 * SQUARE_SIZE_MM;
            let sq_bottom = board_top - (rank_index as f32 + 1.0) * SQUARE_SIZE_MM;
            let offset_x = (SQUARE_SIZE_MM - graphic.width_mm) / 2.0;
            let offset_y = (SQUARE_SIZE_MM - graphic.height_mm) / 2.0;

            graphic.xobject.clone().add_to_layer(
                layer,
                SvgTransform {
                    translate_x: Some(Mm(sq_left + offset_x).into()),
                    translate_y: Some(Mm(sq_bottom + offset_y).into()),
                    rotate: None,
                    scale_x: None,
                    scale_y: None,
                    dpi: Some(graphic.dpi),
                },
            );
        }
    }

    board_top - BOARD_SIZE_MM - BOARD_GAP_MM
}

/// Draws the moves table: column header ("No." / "White" /
/// "Black" / "Comment") then one line per move number, with
/// vertical separators between the columns (user request,
/// 08/07/2026: "the columns need to be narrower... make a clean
/// table... with a header and columns (with lines)", then
/// "I don't want a horizontal line in the table, just
/// vertical lines to separate the columns" — no horizontal line between
/// the rows nor below the header). The "Comment" column stays empty when
/// printed: it is meant for handwritten annotation on paper.
///
/// Drawn starting from the Y coordinate `top_y` (typically the value
/// returned by [`draw_board`]).
///
/// Automatic pagination: if a row no longer fits above the bottom
/// margin of the current page, a new A4 page is added to the document
/// (`doc.add_page`), the column header (and its separators) is repeated
/// at the top of this new page, and drawing continues there.
/// `initial_page`/`initial_layer` are the indices of the first page (the one
/// already containing the header and the board) — drawing
/// starts there before switching to any following pages.
///
/// Draws nothing if `moves` is empty (game with no move played).
// Clippy: `#[allow(too_many_lines)]` — deliberately monolithic function
// (header + grid + move loop + pagination), splitting it into
// sub-functions would separate strongly coupled steps (same
// `draw_column_headers`/`draw_column_dividers` closures, same column coordinates) with
// no real readability gain.
#[allow(clippy::too_many_lines)]
fn draw_moves_table(
    doc: &printpdf::PdfDocumentReference,
    initial_page: printpdf::PdfPageIndex,
    initial_layer: printpdf::PdfLayerIndex,
    top_y: f32,
    title_font: &IndirectFontRef,
    body_font: &IndirectFontRef,
    moves: &[crate::MoveRow],
) {
    if moves.is_empty() {
        return;
    }

    let number_col_x = MARGIN_MM;
    let white_col_x = number_col_x + MOVES_NUMBER_COL_WIDTH_MM;
    let black_col_x = white_col_x + MOVES_MOVE_COL_WIDTH_MM;
    let comment_col_x = black_col_x + MOVES_MOVE_COL_WIDTH_MM;
    let table_right = comment_col_x + MOVES_COMMENT_COL_WIDTH_MM;
    // X coordinates of the 5 vertical grid lines: left edge, between
    // each pair of columns, right edge.
    let col_lines_x = [number_col_x, white_col_x, black_col_x, comment_col_x, table_right];

    let draw_column_headers = |layer: &PdfLayerReference, y: f32| {
        let pad = MOVES_CELL_TEXT_PADDING_MM;
        layer.use_text("N°", MOVES_FONT_SIZE, Mm(number_col_x + pad), Mm(y), title_font);
        layer.use_text("Blancs", MOVES_FONT_SIZE, Mm(white_col_x + pad), Mm(y), title_font);
        layer.use_text("Noirs", MOVES_FONT_SIZE, Mm(black_col_x + pad), Mm(y), title_font);
        layer.use_text(
            "Commentaire",
            MOVES_FONT_SIZE,
            Mm(comment_col_x + pad),
            Mm(y),
            title_font,
        );
    };

    // Draws only the vertical separators between columns for a
    // table row (header or move), whose text has been (or will be)
    // written at baseline `y` — no horizontal line between the
    // rows nor below the header (explicit user request: "I don't
    // want a horizontal line in the table, just
    // vertical lines to separate the columns"). The top of the segment is offset
    // by `MOVES_ROW_TOP_PADDING_MM` above `y` — thanks to this
    // constant offset, the bottom of a row always coincides exactly with the top
    // of the next one, so that consecutive vertical segments
    // form a single continuous line across the whole height of the table,
    // even across a page break.
    let draw_column_dividers = |layer: &PdfLayerReference, y: f32| {
        let row_top = y + MOVES_ROW_TOP_PADDING_MM;
        let row_bottom = row_top - MOVES_ROW_HEIGHT_MM;
        for &x in &col_lines_x {
            layer.add_line(Line {
                points: vec![
                    (Point::new(Mm(x), Mm(row_top)), false),
                    (Point::new(Mm(x), Mm(row_bottom)), false),
                ],
                is_closed: false,
            });
        }
    };

    let mut layer = doc.get_page(initial_page).get_layer(initial_layer);
    let mut y = top_y;

    // Fix (08/07/2026): `draw_board` leaves the fill color
    // of the last drawn square (brown/beige) active on this same layer —
    // `use_text` uses this current fill color for the text,
    // which made the first moves unreadable on the first page
    // (reported by the user). Explicit reset to black before any text.
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    // Black grid, 0 pt thickness — special value documented by
    // `printpdf` ("0.0 does not make the line disappear, it shows up as 1px
    // on every device"), the sharpest rendering for thin table
    // lines.
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_thickness(0.0);

    draw_column_headers(&layer, y);
    draw_column_dividers(&layer, y);
    y -= MOVES_ROW_HEIGHT_MM;

    for mv in moves {
        if y - MOVES_ROW_HEIGHT_MM < MARGIN_MM {
            let (new_page, new_layer_idx) =
                doc.add_page(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Suite des coups");
            layer = doc.get_page(new_page).get_layer(new_layer_idx);
            y = PAGE_HEIGHT_MM - MARGIN_MM;
            // Same precaution as above: new layer, but the
            // black/thickness are set explicitly rather than relying on
            // undocumented default values.
            layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            layer.set_outline_thickness(0.0);
            draw_column_headers(&layer, y);
            draw_column_dividers(&layer, y);
            y -= MOVES_ROW_HEIGHT_MM;
        }

        let pad = MOVES_CELL_TEXT_PADDING_MM;
        layer.use_text(
            mv.number_str.as_str(),
            MOVES_FONT_SIZE,
            Mm(number_col_x + pad),
            Mm(y),
            body_font,
        );
        layer.use_text(
            mv.white_san.as_str(),
            MOVES_FONT_SIZE,
            Mm(white_col_x + pad),
            Mm(y),
            body_font,
        );
        if !mv.black_san.is_empty() {
            layer.use_text(
                mv.black_san.as_str(),
                MOVES_FONT_SIZE,
                Mm(black_col_x + pad),
                Mm(y),
                body_font,
            );
        }
        draw_column_dividers(&layer, y);
        y -= MOVES_ROW_HEIGHT_MM;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEN of the starting position, used as the default `final_fen`
    /// in most of these tests (drawn since Step 3).
    const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn sample_move_row(number: &str, white: &str, black: &str) -> crate::MoveRow {
        crate::MoveRow {
            number_str: number.into(),
            white_san: white.into(),
            black_san: black.into(),
            white_ply: 0,
            black_ply: 1,
            white_from_book: false,
            black_from_book: false,
            // PHASE 16, Step 4: no variations in the PDF (feature
            // not planned as of today) — fields always empty here.
            white_variations: slint::SharedString::default(),
            black_variations: slint::SharedString::default(),
            // PHASE 16, Step 6.1: node identifiers / NAG — no use
            // in the PDF export (no context menu on paper), neutral
            // default values.
            white_node_id: -1,
            black_node_id: -1,
            white_nag: slint::SharedString::default(),
            black_nag: slint::SharedString::default(),
            white_variation_node_id: -1,
            black_variation_node_id: -1,
            // PHASE 16, Step 6.3: comments — no use in the PDF
            // export (no inline editing on paper), neutral values.
            white_comment: slint::SharedString::default(),
            black_comment: slint::SharedString::default(),
            // PHASE 70: move category (syntax highlighting) — no
            // use in the PDF export (no color on the printed text),
            // neutral value (0 = normal move) by default.
            white_move_kind: 0,
            black_move_kind: 0,
        }
    }

    #[test]
    fn test_build_pdf_bytes_produces_valid_pdf_header() {
        let moves = vec![sample_move_row("1.", "e4", "e5")];
        let info = PrintGameInfo {
            white_name: "Alice".to_owned(),
            black_name: "Bob".to_owned(),
            date: "03/07/2026".to_owned(),
            time_control_label: "5+0".to_owned(),
            result: None,
        };

        let bytes = build_pdf_bytes(STARTPOS_FEN, &moves, &info);

        // A valid PDF file always starts with the "%PDF-" header
        // (PDF specification, "File Structure" section).
        assert!(
            bytes.starts_with(b"%PDF-"),
            "le document généré doit être un PDF valide"
        );
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_build_pdf_bytes_handles_empty_move_list() {
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(STARTPOS_FEN, &[], &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_print_game_info_default_has_no_result() {
        assert_eq!(PrintGameInfo::default().result, None);
    }

    // ── Step 2: header banner ────────────────────────────────────────────────

    /// Creates a minimal document/page/layer/font, reused by the
    /// [`draw_header`] tests to avoid depending on [`build_pdf_bytes`].
    ///
    /// Also returns `doc` (`PdfDocumentReference`): `PdfLayerReference`
    /// is only a weak reference (`Weak`) to the internal document — if
    /// `doc` is dropped (e.g. left inside this helper without
    /// being returned), `printpdf` panics internally (`Weak::upgrade()` on
    /// `None`) as soon as `use_text` is first called on the returned layer.
    /// The caller must therefore keep the returned `doc` alive (even without
    /// using it directly) for the entire duration of the test.
    fn test_layer_and_fonts() -> (
        printpdf::PdfDocumentReference,
        PdfLayerReference,
        IndirectFontRef,
        IndirectFontRef,
    ) {
        let (doc, page1, layer1) =
            PdfDocument::new("test", Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Layer 1");
        let layer = doc.get_page(page1).get_layer(layer1);
        let title_font = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let body_font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
        (doc, layer, title_font, body_font)
    }

    #[test]
    fn test_draw_header_returns_lower_y_when_result_present() {
        let (_doc, layer, title_font, body_font) = test_layer_and_fonts();

        let info_no_result = PrintGameInfo {
            white_name: "Alice".to_owned(),
            black_name: "Bob".to_owned(),
            date: "03/07/2026".to_owned(),
            time_control_label: "5+0".to_owned(),
            result: None,
        };
        let info_with_result = PrintGameInfo {
            result: Some("1-0".to_owned()),
            ..info_no_result.clone()
        };

        let y_no_result = draw_header(&layer, &title_font, &body_font, &info_no_result);
        let y_with_result = draw_header(&layer, &title_font, &body_font, &info_with_result);

        assert!(
            y_with_result < y_no_result,
            "une ligne résultat en plus doit faire descendre davantage le curseur Y \
             (y_no_result = {y_no_result}, y_with_result = {y_with_result})"
        );
    }

    #[test]
    fn test_draw_header_stays_within_page_bounds() {
        let (_doc, layer, title_font, body_font) = test_layer_and_fonts();
        let info = PrintGameInfo {
            white_name: "Alice".to_owned(),
            black_name: "Bob".to_owned(),
            date: "03/07/2026".to_owned(),
            time_control_label: "5+0".to_owned(),
            result: Some("1-0".to_owned()),
        };

        let y = draw_header(&layer, &title_font, &body_font, &info);

        assert!(y > 0.0, "le bandeau d'en-tête ne doit pas déborder en bas de page");
        assert!(y < PAGE_HEIGHT_MM, "le curseur Y doit avoir avancé sous le haut de page");
    }

    #[test]
    fn test_build_pdf_bytes_header_with_special_characters_does_not_panic() {
        // Parentheses, apostrophe, em dash, French accents: all
        // potentially tricky cases for PDF string escaping
        // or standard font encoding — this test guarantees the absence of a
        // panic, not the exact visual rendering (see follow-up note at the top of the
        // module).
        let info = PrintGameInfo {
            white_name: "Joueur (Blancs)".to_owned(),
            black_name: "Stockfish — d'ouverture accentuée : éàçè".to_owned(),
            date: "03/07/2026".to_owned(),
            time_control_label: "5+0".to_owned(),
            result: Some("1-0".to_owned()),
        };

        let bytes = build_pdf_bytes(STARTPOS_FEN, &[], &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_build_pdf_bytes_with_full_header_is_valid_pdf() {
        let info = PrintGameInfo {
            white_name: "Alice".to_owned(),
            black_name: "Stockfish 17".to_owned(),
            date: "03/07/2026".to_owned(),
            time_control_label: "5+0".to_owned(),
            result: Some("0-1".to_owned()),
        };
        let bytes = build_pdf_bytes(STARTPOS_FEN, &[], &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    // ── Step 3: board (grid + pieces) ────────────────────────────────────────

    #[test]
    fn test_parse_fen_placement_startpos_black_rook_a8_and_white_king_e1() {
        let grid = parse_fen_placement(STARTPOS_FEN);

        // grid[0] = rank 8 (top); column 0 = file a.
        assert_eq!(grid[0][0], Some(FenPiece { is_white: false, kind: 'R' }), "a8 = tour noire");
        assert_eq!(grid[0][4], Some(FenPiece { is_white: false, kind: 'K' }), "e8 = roi noir");
        // grid[7] = rank 1 (bottom).
        assert_eq!(grid[7][0], Some(FenPiece { is_white: true, kind: 'R' }), "a1 = tour blanche");
        assert_eq!(grid[7][4], Some(FenPiece { is_white: true, kind: 'K' }), "e1 = roi blanc");
        // e4 (rank 4 → rank_index 4) must be empty at the starting position.
        assert_eq!(grid[4][4], None, "e4 doit être vide en position de départ");
    }

    #[test]
    fn test_parse_fen_placement_after_e4_shows_white_pawn_on_e4() {
        let fen_after_e4 = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let grid = parse_fen_placement(fen_after_e4);

        assert_eq!(
            grid[4][4],
            Some(FenPiece { is_white: true, kind: 'P' }),
            "e4 doit contenir un pion blanc après 1.e4"
        );
        // e2 (rank 2 → rank_index 6) must now be empty.
        assert_eq!(grid[6][4], None, "e2 doit être vide après 1.e4");
    }

    #[test]
    fn test_parse_fen_placement_empty_string_does_not_panic_and_is_all_none() {
        let grid = parse_fen_placement("");
        for rank in &grid {
            for square in rank {
                assert_eq!(*square, None);
            }
        }
    }

    #[test]
    fn test_parse_fen_placement_malformed_input_does_not_panic() {
        // String with no valid FEN structure: must never panic, whatever
        // output is produced.
        let _ = parse_fen_placement("n'importe quoi, pas un FEN valide !");
        let _ = parse_fen_placement("////////");
        let _ = parse_fen_placement("8/8/8/8/8/8/8/8/8/8/8");
    }

    #[test]
    fn test_piece_svg_source_covers_all_12_combinations_with_distinct_content() {
        let kinds = ['K', 'Q', 'R', 'B', 'N', 'P'];
        let mut sources = std::collections::HashSet::new();

        for is_white in [true, false] {
            for kind in kinds {
                let src = piece_svg_source(FenPiece { is_white, kind });
                assert!(!src.is_empty());
                assert!(src.contains("<svg"), "doit être un contenu SVG valide");
                sources.insert(src);
            }
        }

        assert_eq!(sources.len(), 12, "les 12 pièces doivent utiliser 12 SVG distincts");
    }

    #[test]
    fn test_draw_board_returns_y_below_board_with_expected_gap() {
        let (_doc, layer, _title_font, _body_font) = test_layer_and_fonts();
        let top_y = 200.0_f32;

        let y = draw_board(&layer, top_y, STARTPOS_FEN);

        let expected = top_y - BOARD_GAP_MM - BOARD_SIZE_MM - BOARD_GAP_MM;
        assert!(
            (y - expected).abs() < f32::EPSILON,
            "y = {y}, attendu = {expected}"
        );
    }

    #[test]
    fn test_build_pdf_bytes_with_starting_position_is_valid_pdf_and_embeds_pieces() {
        let info = PrintGameInfo::default();
        let bytes_with_board = build_pdf_bytes(STARTPOS_FEN, &[], &info);
        assert!(bytes_with_board.starts_with(b"%PDF-"));

        // The starting board has 32 pieces across 12 distinct embedded
        // SVGs: the resulting PDF must be noticeably larger
        // than a document containing only a header banner (simple
        // regression check confirming that drawing the board does have an effect).
        let empty_board_fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        let bytes_empty_board = build_pdf_bytes(empty_board_fen, &[], &info);
        assert!(bytes_empty_board.starts_with(b"%PDF-"));
        assert!(
            bytes_with_board.len() > bytes_empty_board.len(),
            "un échiquier avec pièces doit produire un PDF plus volumineux \
             qu'un échiquier vide"
        );
    }

    #[test]
    fn test_build_pdf_bytes_with_empty_board_does_not_panic() {
        let empty_board_fen = "8/8/8/8/8/8/8/8 w - - 0 1";
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(empty_board_fen, &[], &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    // ── Step 4: moves table (pagination) ─────────────────────────────────────

    /// Generates `count` fake move rows ("1." to "count.", e4/e5),
    /// enough for the pagination tests — the exact SAN content doesn't
    /// matter here, only the number of rows counts.
    fn sample_moves(count: usize) -> Vec<crate::MoveRow> {
        (1..=count)
            .map(|n| sample_move_row(&format!("{n}."), "e4", "e5"))
            .collect()
    }

    #[test]
    fn test_build_pdf_bytes_with_few_moves_is_valid_pdf() {
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(STARTPOS_FEN, &sample_moves(5), &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_build_pdf_bytes_with_many_moves_triggers_pagination_without_panic() {
        // Far more rows than a single A4 page could hold
        // below the board (see `BOARD_SIZE_MM`) — forces `draw_moves_table` to
        // add extra pages via `doc.add_page`. The sole goal of this
        // test is the absence of a panic and a structurally valid PDF:
        // the actual visual rendering cannot be checked without a PDF reader (see
        // follow-up note at the top of the module).
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(STARTPOS_FEN, &sample_moves(80), &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_build_pdf_bytes_more_moves_produce_larger_pdf() {
        let info = PrintGameInfo::default();
        let bytes_few = build_pdf_bytes(STARTPOS_FEN, &sample_moves(2), &info);
        let bytes_many = build_pdf_bytes(STARTPOS_FEN, &sample_moves(80), &info);
        assert!(
            bytes_many.len() > bytes_few.len(),
            "davantage de coups (et de pages) doit produire un PDF plus volumineux"
        );
    }

    #[test]
    fn test_build_pdf_bytes_with_pending_black_move_does_not_panic() {
        // Last white move played, Black has not answered yet: `black_san`
        // empty — real case produced by `build_move_rows` mid-game.
        let moves = vec![sample_move_row("1.", "e4", "")];
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(STARTPOS_FEN, &moves, &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_build_pdf_bytes_with_empty_moves_list_skips_table_but_stays_valid() {
        let info = PrintGameInfo::default();
        let bytes = build_pdf_bytes(STARTPOS_FEN, &[], &info);
        assert!(bytes.starts_with(b"%PDF-"));
    }

    // ── Step 5: today's date (header banner, no external dependency) ────────

    #[test]
    fn test_civil_from_days_unix_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn test_civil_from_days_day_after_epoch() {
        assert_eq!(civil_from_days(1), (1970, 1, 2));
    }

    #[test]
    fn test_civil_from_days_end_of_january() {
        // Day 31 (0-indexed starting from day 0 = January 1st) = February 1st,
        // January having 31 days.
        assert_eq!(civil_from_days(31), (1970, 2, 1));
    }

    #[test]
    fn test_civil_from_days_non_leap_year_rollover() {
        // 1970 is not a leap year (365 days): day 365 must be
        // January 1st, 1971.
        assert_eq!(civil_from_days(365), (1971, 1, 1));
    }

    #[test]
    fn test_civil_from_days_known_reference_2000_01_01() {
        // Well-known reference value: January 1st, 2000 corresponds to
        // day 10957 since the Unix epoch (1970-01-01).
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }

    #[test]
    fn test_civil_from_days_leap_year_feb_29() {
        // 2000 is a leap year (divisible by 400): the 60th day of the year
        // (day 59, 0-indexed: 31 days of January + 29 days of February)
        // must be February 29th, not March 1st.
        assert_eq!(civil_from_days(10_957 + 31 + 28), (2000, 2, 29));
        assert_eq!(civil_from_days(10_957 + 31 + 29), (2000, 3, 1));
    }

    #[test]
    fn test_today_date_string_has_expected_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 10, "format attendu JJ/MM/AAAA (10 caractères) : {date}");
        let parts: Vec<&str> = date.split('/').collect();
        assert_eq!(parts.len(), 3, "trois segments séparés par '/' attendus : {date}");
        assert_eq!(parts[0].len(), 2, "jour sur 2 chiffres : {date}");
        assert_eq!(parts[1].len(), 2, "mois sur 2 chiffres : {date}");
        assert_eq!(parts[2].len(), 4, "année sur 4 chiffres : {date}");
        // Plausible year (development of this feature
        // started in 2026; wide margin to stay valid for a long time).
        let year: u32 = parts[2].parse().expect("année numérique");
        assert!(year >= 2026, "année suspecte : {date}");
    }

    // ── Step 6: full end-to-end scenario ─────────────────────────────────────

    #[test]
    fn test_build_pdf_bytes_full_realistic_game_end_to_end() {
        // Combines all the building blocks of Steps 1 to 5 into a single generation:
        // full header banner (players, date, time control, result),
        // non-trivial final position (midgame, not the starting
        // position), and a non-empty move list including a row with a
        // missing black move (unfinished game). Goal: detect any
        // integration regression that would not be visible in the
        // per-function unit tests (`draw_header`/`draw_board`/`draw_moves_table`
        // tested so far separately or only in pairs).
        let mid_game_fen = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";
        let moves = vec![
            sample_move_row("1.", "e4", "e5"),
            sample_move_row("2.", "Nf3", "Nc6"),
            sample_move_row("3.", "Bc4", ""),
        ];
        let info = PrintGameInfo {
            white_name: "Alice".to_owned(),
            black_name: "Stockfish 17".to_owned(),
            date: today_date_string(),
            time_control_label: "5+0".to_owned(),
            result: None,
        };

        let bytes = build_pdf_bytes(mid_game_fen, &moves, &info);

        assert!(bytes.starts_with(b"%PDF-"), "document final invalide");
        assert!(
            bytes.len() > 1000,
            "un document complet (en-tête + échiquier + tableau des coups) doit être substantiel"
        );
    }

    #[test]
    fn test_build_pdf_bytes_finished_game_with_result_and_many_moves() {
        // Finished game (result set) combined with pagination across
        // several pages — the two most complex mechanisms of
        // PHASE 25 exercised simultaneously.
        let info = PrintGameInfo {
            white_name: "Stockfish 17".to_owned(),
            black_name: "Alice".to_owned(),
            date: today_date_string(),
            time_control_label: "Illimité".to_owned(),
            result: Some("1-0".to_owned()),
        };

        let bytes = build_pdf_bytes(STARTPOS_FEN, &sample_moves(60), &info);

        assert!(bytes.starts_with(b"%PDF-"));
    }
}
