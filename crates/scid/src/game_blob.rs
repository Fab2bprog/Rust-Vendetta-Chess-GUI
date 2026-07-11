//! Decoding a `.sg4`/`.sg5` game blob (extra tags, flags, optional FEN,
//! move list, variations) into a full [`GameState`]
//! (`chess_core::game_tree::GameTree`, not just the main line —
//! see V2 Phase D below).
//!
//! Blob structure, EXACT ORDER (see `si4_specification_fr.txt` §4.2,
//! identical for si5 §4.2):
//!   (a) non-standard PGN tags         -> ignored (skipped)
//!   (b)(c) 1 flag byte + optional FEN -> non-standard position: read and
//!          decoded since V2 Phase C2 (task #22, see below);
//!          `error::GameDecodeError::NonStandardStart` is no longer returned
//!          by this module (kept in the enum for compatibility/Display).
//!   (d) move list (control tokens included, nested variations
//!       decoded since V2 Phase D, task #23, see below)
//!   (e) comment block -> read since the V2 Phase B bugfix (12/07/2026,
//!       task #20, see below); never read before in V1.
//!
//! V1 history (12/07/2026, before the phases below): ignoring
//! NAGs and comments was safe (they are pure markers, with no effect
//! on the position) — but encountering a VARIATION (`ENCODE_START_MARKER`
//! token) then caused decoding of the ENTIRE game to fail rather than
//! attempting to blindly "skip" its bytes: the size of an encoded move (1 or
//! 2 bytes) depends on the TYPE of the piece at the origin
//! square, which is only known by actually replaying the variation on
//! its own board state — exactly what `Game::DecodeVariation`
//! does in the reference implementation (it also decodes variations, it never
//! skips them blindly). That is now the case here too, see V2 Phase D.
//!
//! ## Bugfix 12/07/2026 — piece-index renumbering on capture
//!
//! First test against a real `.si4` file: 33% of games rejected
//! (see `SUIVI_PLAN_ACTION.md`). Diagnosed by cross-checking against `position.cpp`
//! (`Position::DoSimpleMove`, "handle captures" section): the piece index
//! encoded in the high nibble of each byte (`byte >> 4`) is NOT
//! stable throughout the game. On every capture, the index of the
//! captured piece is REASSIGNED to the piece that until then held the
//! highest active index of its side (removal via "swap with the last element"
//! of a variable-size list, `Count[side]` decremented on every capture) — the
//! `standard_piece_list` table in `moves.rs` is therefore only a valid
//! index -> square mapping UNTIL the first capture. Fixed below
//! by exactly replicating this reassignment (`count`/`renumber_on_capture`).
//!
//! ## V2 Phase B (12/07/2026, task #20) — comment decoding
//!
//! Reread `game.cpp` (`decodeComments`, `encodeComments`, `countComments`,
//! `Game::Decode`) before coding — not just the spec, same discipline as the
//! bugfix above. Two discoveries that simplify the implementation:
//!
//! 1. The format theoretically supports a "dense" mode (no
//!    `ENCODE_COMMENT` token in the stream, one string — empty or not — per
//!    move, in order, for games that are mostly commented) BUT
//!    `Game::Encode` FORCES `markComments = true` unconditionally
//!    ("Compatibility: SCID4 requires the markers", `game.cpp` line ~2942) —
//!    this dense mode is therefore never produced by a real encoder. Only the
//!    "sparse" mode (one `ENCODE_COMMENT` token per commented move, a block of
//!    strings terminated by `0x00` in the same order) is implemented here.
//! 2. An `ENCODE_COMMENT` token encountered BEFORE the very first move of
//!    the game ("pregame" comment) still consumes its string from the
//!    final block (so as not to desynchronize the following comments), but
//!    `chess_core` has no notion of a pre-game comment (a
//!    `GameNode` always represents a move played) — this string is therefore
//!    read then silently discarded, the same principle as NAGs outside 1-6
//!    (Phase A2): not corruption, just data that this software does
//!    not yet know how to represent.
//!
//! A read error INSIDE the comment block (truncation) does NOT cancel
//! the game: at this point the moves are already decoded and validated
//! as legal — we simply stop attaching the remaining comments
//! rather than rejecting an otherwise-correct game over missing
//! enrichment data.
//!
//! ## V2 Phase C2 (12/07/2026, task #22) — non-standard starting positions
//!
//! Reread `position.cpp` (`Position::AddPiece`, `Position::ReadFromFEN`) before
//! coding. Non-trivial point discovered: the `moves::
//! standard_piece_list` table (piece index -> starting square) is valid
//! ONLY for the standard starting position. For a custom FEN, the
//! piece indices are assigned by `AddPiece` IN THE ORDER ENCOUNTERED
//! while parsing the FEN (scanning rank 8 -> 1, file a -> h, exactly
//! the natural order of writing a FEN), with one exception: the King is
//! ALWAYS forced to index 0, regardless of its position in this scan
//! order — if it is not the first piece encountered for its side, the
//! piece that already held index 0 is moved to the last free index.
//! Reproduced below by [`build_piece_lists`], which replays this scan
//! directly on the `Position` already parsed by `chess_core` (strictly
//! equivalent to replaying the FEN text itself, without having to
//! reparse it).
//!
//! ## V2 Phase D (13/07/2026, task #23) — real decoding of variations
//!
//! Reread `game.cpp` (`Game::DecodeVariation`, `AddVariation`,
//! `MoveExitVariation`, `MoveForward`) and `bytebuf.h` (`ByteBuffer::nextMove`)
//! before coding — same discipline as for all previous phases.
//! Key discovery: the C++ reference decodes the ENTIRE game (main
//! line and variations nested to any depth) with a SINGLE flat loop over
//! `nextMove`, having `CurrentMove`/`CurrentPos` follow the structure of
//! the already-built tree (`AddVariation` steps back one move then
//! descends into a new child branch, `MoveExitVariation` goes back up to
//! the parent) rather than by recursion — not a
//! coincidence: `chess_core::game_tree::GameTree` had already made the same
//! choice (`flatten_expand`, code audit 04/07/2026) precisely to
//! never risk a stack overflow on a pathologically nested game.
//! [`decode_variation_tree`] reproduces this choice with an explicit
//! stack (`Vec<VarFrame>`) rather than a recursive call: a variation does
//! NOT step back by "undoing" the last move played (`Position::UndoSimpleMove`,
//! non-trivial to replicate for `list`/`count` because of the swap-based
//! removal from the 12/07/2026 bugfix — a swap is not trivially invertible
//! without extra information), but by restarting from a SNAPSHOT
//! (cloned `pos`/`list`/`count`) taken right before that move was played —
//! strictly equivalent, without having to write the inverse of the
//! renumbering on capture.
//!
//! Important: moves decoded within a variation NEVER go through
//! `GameState::play` (which would make that variation the active
//! line/`path` — see `chess_core::history::History::branch_at`, reserved
//! for interactive user exploration), but go directly through
//! `chess_core::game_tree::GameTree::add_move`: the main line of the
//! source SCID file must remain `children[0]`/the main line of
//! the imported tree, exactly as it was in the original file.
//! All the `list`/`count` update logic (renumbering on
//! capture, rook-index lookup on castling) is SHARED with the main
//! line via [`resolve_move`]/[`apply_piece_lists`] (extracted with no
//! behavior change from the old body of [`apply_one_move`]):
//! a single implementation, never two copies at risk of diverging.

use crate::bytes::BeReader;
use crate::error::GameDecodeError;
use crate::moves::{self, DecodedMove};
use chess_core::game::GameState;
use chess_core::game_tree::{GameTree, Nag};
use chess_core::history::MoveRecord;
use chess_core::movegen::generate_legal_moves;
use chess_core::notation::move_to_san;
use chess_core::rules::make_move;
use chess_core::types::chess_move::{Move, MoveKind};
use chess_core::types::piece::{Color, PieceKind};
use chess_core::types::position::Position;
use chess_core::types::square::Square;

const ENCODE_NAG: u8 = 11;
const ENCODE_COMMENT: u8 = 12;
const ENCODE_START_MARKER: u8 = 13;
const ENCODE_END_MARKER: u8 = 14;
const ENCODE_END_GAME: u8 = 15;

/// Decodes the main line of a `.sg4`/`.sg5` blob and returns the resulting
/// [`GameState`] (standard starting position, all moves of
/// the main line played). The game result (`GameState::result`)
/// is NOT set here (it comes from the `IndexEntry`, not the blob) — see
/// `pgn_build.rs`.
///
/// # Errors
/// See the variants of [`GameDecodeError`].
pub fn decode_mainline(blob: &[u8]) -> Result<GameState, GameDecodeError> {
    let mut r = BeReader::new(blob);

    skip_extra_tags(&mut r)?;

    let flags = r
        .read_u8()
        .map_err(|_| GameDecodeError::BadMoveStream("octet de flags de partie manquant"))?;

    // V2 Phase C2 (12/07/2026, task #22): bit 0 = non-standard starting
    // position (§4.4) — the FEN string that follows is read and becomes the
    // initial position, instead of `GameState::new()` (standard position). The
    // piece indices (`list`/`count`) are then derived from THIS position
    // via `build_piece_lists`, not from `moves::standard_piece_list` (see the
    // module doc for the full reasoning).
    let (mut game, mut list, mut count) = if flags & 0x01 != 0 {
        let fen = r
            .read_terminated_cstr()
            .map_err(|_| GameDecodeError::BadMoveStream("FEN de départ non standard manquant"))?;
        let game = GameState::from_fen(&fen)
            .map_err(|_| GameDecodeError::BadMoveStream("FEN de départ non standard invalide"))?;
        let (list, count) = build_piece_lists(game.position())?;
        (game, list, count)
    } else {
        let game = GameState::new();
        let list = [
            moves::standard_piece_list(Color::White),
            moves::standard_piece_list(Color::Black),
        ];
        // Number of active pieces per side: `list[side][0..count[side]]`
        // is the only up-to-date range, the rest becomes stale as
        // captures happen (see the 12/07/2026 bugfix documented at the top of this file).
        let count: [usize; 2] = [16, 16];
        (game, list, count)
    };

    // V2 Phase B (12/07/2026, task #20): an `ENCODE_COMMENT` token encountered
    // at position `i` of this vector corresponds exactly to the `i`-th
    // string of the final comment block (§4.6, "sparse" mode — see the
    // module doc). `Some(node_id)` = move to attach the text to;
    // `None` = pregame comment (before the first move), with no move
    // to attach it to in `chess_core`'s data model — its
    // string is still consumed from the block so as not to desynchronize the
    // following comments, just discarded after reading.
    let mut comment_targets: Vec<Option<usize>> = Vec::new();

    // V2 Phase D (13/07/2026, task #23): snapshot (position + piece
    // lists) taken right BEFORE each move of the main line — serves as
    // the starting point for a possible variation on THIS move (see the
    // module doc and [`decode_variation_tree`]). `None` as long as no move has
    // been played yet (a variation there would be a corrupted stream, detected by
    // the absence of `last_node_id`).
    let mut pre_move: Option<(Position, [[Square; 16]; 2], [usize; 2])> = None;

    loop {
        let byte = r
            .read_u8()
            .map_err(|_| GameDecodeError::BadMoveStream("fin de flux de coups inattendue"))?;

        match byte {
            ENCODE_NAG => {
                let code = r
                    .read_u8()
                    .map_err(|_| GameDecodeError::BadMoveStream("code NAG manquant"))?;
                apply_nag(&mut game, code);
            }
            ENCODE_COMMENT => {
                // Presence marker only (no inline data): the
                // actual text is read after `ENCODE_END_GAME`, see below.
                comment_targets.push(game.history().last_node_id());
            }
            ENCODE_START_MARKER => {
                let Some((snap_pos, snap_list, snap_count)) = pre_move.clone() else {
                    return Err(GameDecodeError::BadMoveStream(
                        "variante rencontrée avant tout coup de la ligne principale",
                    ));
                };
                let Some(cur_id) = game.history().last_node_id() else {
                    return Err(GameDecodeError::BadMoveStream(
                        "variante rencontrée avant tout coup de la ligne principale",
                    ));
                };
                let parent = game.history().tree().node(cur_id).and_then(|n| n.parent);
                decode_variation_tree(
                    game.history_mut().tree_mut(),
                    snap_pos,
                    snap_list,
                    snap_count,
                    parent,
                    &mut r,
                    &mut comment_targets,
                )?;
            }
            ENCODE_END_MARKER => {
                return Err(GameDecodeError::BadMoveStream(
                    "marqueur de fin de variante en dehors d'une variante",
                ));
            }
            ENCODE_END_GAME => break,
            _ => {
                pre_move = Some((game.position().clone(), list, count));
                apply_one_move(&mut game, &mut list, &mut count, byte, &mut r)?;
            }
        }
    }

    apply_comments(&mut game, &mut r, comment_targets);

    Ok(game)
}

/// V2 Phase C2 (12/07/2026, task #22): builds `list`/`count` (the same
/// structures as for the standard position, see the 12/07/2026 bugfix at the
/// top of this file) for a NON-standard starting position, by replaying on
/// `position` the index-assignment algorithm of `Position::AddPiece`
/// (`position.cpp`): scanning rank 8 -> 1, file a -> h (the natural
/// order for parsing a FEN), King ALWAYS forced to index 0 (the piece that
/// already held this index, if there was one, is moved to the last
/// free index).
///
/// # Errors
/// [`GameDecodeError::BadMoveStream`] if a side has more than 16 pieces (a
/// hard bound from `[Square; 16]`, identical to the `Count[c] > 15` rejection
/// of the reference implementation) — protects against an out-of-bounds
/// access panic on a malformed FEN rather than trusting external data.
fn build_piece_lists(position: &Position) -> Result<([[Square; 16]; 2], [usize; 2]), GameDecodeError> {
    let mut list = [[Square::new(0, 0); 16]; 2];
    let mut count: [usize; 2] = [0, 0];

    for rank in (0..8u8).rev() {
        for file in 0..8u8 {
            let sq = Square::new(file, rank);
            let Some(piece) = position.piece_at(sq) else { continue };
            let side_idx = usize::from(piece.color == Color::Black);

            if count[side_idx] >= 16 {
                return Err(GameDecodeError::BadMoveStream(
                    "plus de 16 pièces d'un camp dans le FEN de départ",
                ));
            }

            if piece.kind == PieceKind::King && count[side_idx] > 0 {
                // The King must occupy index 0: the piece that was
                // already there (encountered before it in the scan, therefore
                // placed by the `else` branch below in a previous
                // iteration) is moved to the last free index before the King
                // takes its place.
                let displaced = list[side_idx][0];
                list[side_idx][count[side_idx]] = displaced;
                list[side_idx][0] = sq;
            } else if piece.kind == PieceKind::King {
                // First piece encountered for this side: index 0 is
                // still free, the King takes it directly.
                list[side_idx][0] = sq;
            } else {
                // Non-King piece: added normally at the next free
                // index (will be moved later if the King arrives afterward and
                // this piece occupies index 0).
                list[side_idx][count[side_idx]] = sq;
            }
            count[side_idx] += 1;
        }
    }

    Ok((list, count))
}

/// V2 Phase B (12/07/2026, task #20): reads the comment block (§4.6) —
/// exactly `comment_targets.len()` strings terminated by `0x00`, in the
/// same order as the `ENCODE_COMMENT` tokens encountered while decoding
/// the move stream — and attaches each to the corresponding move.
///
/// A missing/truncated string simply stops the attachment of the
/// remaining comments (see module doc): the moves are already decoded
/// and validated as legal at this point, an otherwise-correct game is not
/// rejected over a problem purely in the enrichment data.
fn apply_comments(game: &mut GameState, r: &mut BeReader<'_>, comment_targets: Vec<Option<usize>>) {
    for target in comment_targets {
        let Ok(text) = r.read_terminated_cstr() else { break };
        let Some(node_id) = target else { continue };
        if let Some(node) = game.history_mut().tree_mut().node_mut(node_id) {
            node.comment = Some(text);
        }
    }
}

/// V2 Phase A2 (12/07/2026, task #19): attaches the actual value of a NAG
/// to the LAST move played, rather than discarding it as V1 did
/// (`r.skip(1)`) — the NAG always immediately follows the move it annotates
/// in the stream (same convention as `ENCODE_COMMENT`, already handled this way).
///
/// `chess_core::game_tree::Nag` only covers the 6 traditional glyphs
/// (PGN codes 1-6, `!!`/`!`/`!?`/`?!`/`?`/`??`) — a CLOSED type, shared by
/// the whole application (manual annotation context menu). Extending it to
/// the full NAG standard (up to `$255`, positional evaluation symbols
/// etc.) would touch this shared type for a gain outside the deliberately
/// restricted scope of this phase ("quick win, near-zero risk"
/// — see `SUIVI_PLAN_ACTION.md`). A code outside 1-6, or encountered before any
/// move played (edge case, never observed), is therefore simply ignored: this
/// is not corruption, just an annotation that this software does not
/// yet know how to represent.
fn apply_nag(game: &mut GameState, code: u8) {
    let Some(nag) = Nag::from_code(code) else { return };
    let Some(id) = game.history().last_node_id() else { return };
    if let Some(node) = game.history_mut().tree_mut().node_mut(id) {
        node.nag = Some(nag);
    }
}

/// V2 Phase D (13/07/2026, task #23): result of resolving a
/// move byte into a full legal move, BEFORE actually applying it — a step
/// common to the main line ([`apply_one_move`]) and to variations
/// ([`decode_variation_tree`]), extracted with no behavior change from
/// the old single body of `apply_one_move` (see the module doc).
struct ResolvedMove {
    color: Color,
    side_idx: usize,
    piece_index: usize,
    candidate: Move,
    captured_square: Option<Square>,
    castle_kingside: bool,
    castle_queenside: bool,
}

/// Decodes the move byte `byte` (reading a 2nd byte if needed, diagonal
/// Queen move) into a [`ResolvedMove`] legal in `pos`, without modifying
/// anything (neither `pos`, nor `list`/`count`) — the caller then decides
/// how to actually apply the move (see [`apply_one_move`] for the
/// main line, [`decode_variation_tree`] for a variation).
///
/// `ply` is only used for diagnostics (`GameDecodeError::IllegalMove`) —
/// move number in the main line, or a local approximation within the
/// variation with no global meaning (see the caller).
fn resolve_move(
    pos: &Position,
    list: &[[Square; 16]; 2],
    byte: u8,
    r: &mut BeReader<'_>,
    ply: usize,
) -> Result<ResolvedMove, GameDecodeError> {
    let color = pos.side_to_move;
    let side_idx = usize::from(color == Color::Black);
    let piece_index = usize::from(byte >> 4);

    let from = list[side_idx][piece_index];
    let piece = pos
        .piece_at(from)
        .ok_or(GameDecodeError::BadMoveStream("aucune pièce sur la case déduite de l'indice"))?;
    if piece.color != color {
        return Err(GameDecodeError::BadMoveStream(
            "la pièce déduite de l'indice n'appartient pas au camp au trait",
        ));
    }

    let decoded = moves::decode_move(color, piece.kind, from, byte, r)?;

    let (real_to, promotion, castle_kingside, castle_queenside) = match decoded {
        DecodedMove::Normal { to, promotion } => (to, promotion, false, false),
        DecodedMove::CastleKingside  => (Square::new(6, from.rank()), None, true, false),
        DecodedMove::CastleQueenside => (Square::new(2, from.rank()), None, false, true),
        DecodedMove::NullMove => return Err(GameDecodeError::NullMove),
    };

    let candidate = generate_legal_moves(pos)
        .into_iter()
        .find(|m| m.from == from && m.to == real_to && m.promotion == promotion)
        .ok_or(GameDecodeError::IllegalMove { ply })?;

    // Square actually captured (if a capture), determined BEFORE playing the
    // move — the board must still reflect the pre-move state. En
    // passant capture: it is not `real_to` but the square of the captured pawn
    // (same file as the destination, same rank as the origin).
    let captured_square = if candidate.kind == MoveKind::EnPassant {
        Some(Square::new(real_to.file(), from.rank()))
    } else if pos.piece_at(real_to).is_some() {
        Some(real_to)
    } else {
        None
    };

    Ok(ResolvedMove {
        color,
        side_idx,
        piece_index,
        candidate,
        captured_square,
        castle_kingside,
        castle_queenside,
    })
}

/// Updates `list`/`count` after a [`ResolvedMove`] has actually been
/// played (square of the moving piece, Rook on castling, renumbering on
/// capture — see the 12/07/2026 bugfix at the top of this file and the V2
/// Phase C2 note on `castling_rook`). Shared by the main line and
/// variations: a single implementation of this delicate logic.
fn apply_piece_lists(
    list: &mut [[Square; 16]; 2],
    count: &mut [usize; 2],
    resolved: &ResolvedMove,
) -> Result<(), GameDecodeError> {
    let side_idx = resolved.side_idx;
    list[side_idx][resolved.piece_index] = resolved.candidate.to;

    if resolved.castle_kingside || resolved.castle_queenside {
        // V2 Phase C2 (12/07/2026, task #22): the Rook's index is
        // NO LONGER assumed fixed (1/7) — see `moves::castling_rook`'s doc —
        // it is looked up by searching for its origin square in the
        // active range of `list`, which remains correct both in the
        // standard position and in a custom starting position.
        let (rook_from, rook_to) = moves::castling_rook(resolved.color, resolved.castle_kingside);
        let rook_idx = (0..count[side_idx])
            .find(|&i| list[side_idx][i] == rook_from)
            .ok_or(GameDecodeError::BadMoveStream(
                "tour de roque introuvable dans la liste des pièces (désynchronisation)",
            ))?;
        list[side_idx][rook_idx] = rook_to;
    }

    // Bugfix 12/07/2026: removal via "swap with the last active element",
    // exactly like `Position::DoSimpleMove` (capture section) — the index
    // of the captured piece is reassigned to the piece that until then held
    // the highest active index of the opposing side, then that side's active
    // piece count decreases by 1.
    if let Some(captured_square) = resolved.captured_square {
        let enemy_idx = 1 - side_idx;
        let captured_num = (0..count[enemy_idx])
            .find(|&i| list[enemy_idx][i] == captured_square)
            .ok_or(GameDecodeError::BadMoveStream(
                "case capturée introuvable dans la liste de pièces (désynchronisation)",
            ))?;
        count[enemy_idx] -= 1;
        list[enemy_idx][captured_num] = list[enemy_idx][count[enemy_idx]];
    }

    Ok(())
}

/// Decodes and plays a single move of the main line, updating the
/// `list` table (piece index -> current square, per side — see
/// `moves::standard_piece_list`) as well as `count` (number of active
/// pieces per side, see the 12/07/2026 bugfix documented at the top of this file).
fn apply_one_move(
    game: &mut GameState,
    list: &mut [[Square; 16]; 2],
    count: &mut [usize; 2],
    byte: u8,
    r: &mut BeReader<'_>,
) -> Result<(), GameDecodeError> {
    let ply = game.move_count();
    let resolved = resolve_move(game.position(), list, byte, r, ply)?;

    game.play(resolved.candidate).map_err(|_| GameDecodeError::IllegalMove { ply })?;

    apply_piece_lists(list, count, &resolved)
}

/// V2 Phase D (13/07/2026, task #23): state of the "paused" line to
/// push when a variation starts for one of its moves, restored as-is
/// when that variation ends (`ENCODE_END_MARKER`) — see the module
/// doc for the equivalence with `AddVariation`/`MoveExitVariation` of
/// the C++ reference implementation.
struct VarFrame {
    pos: Position,
    list: [[Square; 16]; 2],
    count: [usize; 2],
    current_node_id: Option<usize>,
    pre_move: Option<(Position, [[Square; 16]; 2], [usize; 2])>,
}

/// V2 Phase D (13/07/2026, task #23): decodes recursively (but WITHOUT
/// Rust recursion — see the module doc) the variation subtree that
/// starts at the first `ENCODE_START_MARKER` encountered, up to its
/// matching `ENCODE_END_MARKER` (depth back to 0).
///
/// `pos`/`list`/`count` = snapshot of the position/piece lists right
/// BEFORE the varied move (see [`decode_mainline`], `pre_move` field).
/// `parent_node_id` = tree node the FIRST move of this
/// variation must attach to (the parent of the varied move, not the varied
/// move itself — a variation REPLACES that move, it is not its continuation).
///
/// Never writes to `tree` outside of `add_move`/`node_mut` (never
/// modifies `path`, which does not even exist at this level — see the module
/// doc): the "main line" node in `chess_core`'s sense remains
/// exactly the one already in place (`children[0]`), the variation moves
/// are added normally afterward under `children[1..]`.
fn decode_variation_tree(
    tree: &mut GameTree,
    mut pos: Position,
    mut list: [[Square; 16]; 2],
    mut count: [usize; 2],
    parent_node_id: Option<usize>,
    r: &mut BeReader<'_>,
    comment_targets: &mut Vec<Option<usize>>,
) -> Result<(), GameDecodeError> {
    let mut current_node_id = parent_node_id;
    let mut pre_move: Option<(Position, [[Square; 16]; 2], [usize; 2])> = None;
    let mut stack: Vec<VarFrame> = Vec::new();

    loop {
        let byte = r
            .read_u8()
            .map_err(|_| GameDecodeError::BadMoveStream("fin de flux de coups inattendue (variante)"))?;

        match byte {
            ENCODE_NAG => {
                let code = r
                    .read_u8()
                    .map_err(|_| GameDecodeError::BadMoveStream("code NAG manquant (variante)"))?;
                if let (Some(nag), Some(id)) = (Nag::from_code(code), current_node_id) {
                    if let Some(node) = tree.node_mut(id) {
                        node.nag = Some(nag);
                    }
                }
            }
            ENCODE_COMMENT => {
                comment_targets.push(current_node_id);
            }
            ENCODE_START_MARKER => {
                let Some((snap_pos, snap_list, snap_count)) = pre_move.clone() else {
                    return Err(GameDecodeError::BadMoveStream(
                        "variante rencontrée avant tout coup de sa propre ligne",
                    ));
                };
                let Some(cur_id) = current_node_id else {
                    return Err(GameDecodeError::BadMoveStream(
                        "variante rencontrée avant tout coup de sa propre ligne",
                    ));
                };
                let parent = tree.node(cur_id).and_then(|n| n.parent);

                // Push the state of THE CURRENT LINE (not that of the
                // new variation) so we can return to it exactly at its
                // `ENCODE_END_MARKER`.
                stack.push(VarFrame {
                    pos: pos.clone(),
                    list,
                    count,
                    current_node_id,
                    pre_move: pre_move.clone(),
                });

                pos = snap_pos;
                list = snap_list;
                count = snap_count;
                current_node_id = parent;
                pre_move = None; // no move played yet in this new branch
            }
            ENCODE_END_MARKER => {
                let Some(frame) = stack.pop() else {
                    // Depth back to 0: this is the end OF THIS variation
                    // (the one for which `decode_variation_tree` was
                    // called) — returns to the caller (`decode_mainline` or
                    // an enclosing variation level).
                    return Ok(());
                };
                pos = frame.pos;
                list = frame.list;
                count = frame.count;
                current_node_id = frame.current_node_id;
                pre_move = frame.pre_move;
            }
            ENCODE_END_GAME => {
                return Err(GameDecodeError::BadMoveStream(
                    "fin de partie rencontrée à l'intérieur d'une variante",
                ));
            }
            _ => {
                // Diagnostics only (see `resolve_move`'s doc): no
                // global notion of "ply" inside a variation, we simply
                // reuse the already-known local depth.
                let ply = current_node_id.and_then(|id| tree.ply_index(id)).map_or(0, |p| p + 1);
                let resolved = resolve_move(&pos, &list, byte, r, ply)?;

                let san = move_to_san(&pos, resolved.candidate);
                let fen_before = pos.to_fen();
                let new_pos = make_move(&pos, resolved.candidate)
                    .map_err(|_| GameDecodeError::IllegalMove { ply })?;

                apply_piece_lists(&mut list, &mut count, &resolved)?;

                let id = tree
                    .add_move(current_node_id, MoveRecord {
                        mv: resolved.candidate,
                        san,
                        fen_before,
                        from_book: false,
                    })
                    .ok_or(GameDecodeError::BadMoveStream(
                        "nœud parent de variante introuvable dans l'arbre (désynchronisation)",
                    ))?;

                pre_move = Some((pos.clone(), list, count));
                current_node_id = Some(id);
                pos = new_pos;
            }
        }
    }
}

/// Skips section (a) of the blob: the list of non-standard PGN tags.
/// See `si4_specification_fr.txt` §4.3 (identical in si5, §4.3, with the
/// minor exception of codes 251-254 which became undefined rather than
/// "reserved unused" — handled here identically and safely in both
/// cases: they are in any case "common" tags with no name to read).
fn skip_extra_tags(r: &mut BeReader<'_>) -> Result<(), GameDecodeError> {
    let trunc = |_| GameDecodeError::BadMoveStream("tags PGN annexes corrompus");
    loop {
        let name_code = r.read_u8().map_err(trunc)?;
        if name_code == 0 {
            return Ok(());
        }
        if name_code == 255 {
            // Legacy case (SCID2 compat.): EventDate packed into 3 raw
            // bytes, with no value-length byte.
            r.skip(3).map_err(trunc)?;
            continue;
        }
        if name_code <= 240 {
            // Custom tag: name_code = length of the tag NAME to skip.
            r.skip(usize::from(name_code)).map_err(trunc)?;
        }
        // 241-254: "common" tag, encoded in this single byte (no name to read).

        let value_len = r.read_u8().map_err(trunc)?;
        r.skip(usize::from(value_len)).map_err(trunc)?;
    }
}
