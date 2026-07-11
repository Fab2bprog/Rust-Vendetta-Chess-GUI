//! PGN (Portable Game Notation) parser and generator.
//!
//! Supports:
//! - Export (PHASE 16, Step 7.1): `GameState` → full PGN string (7
//!   mandatory tags + SAN moves + RAV variations `(...)` at any
//!   depth + NAG `$n` + comments `{...}`), by walking the entire
//!   underlying [`crate::game_tree::GameTree`], not just the
//!   active line — see [`write_siblings`].
//! - Import (PHASE 16, Step 7.2): PGN string → `GameState`, faithfully
//!   reconstructing the entire underlying [`crate::game_tree::GameTree`] —
//!   RAV variations `(...)` at any depth, NAG (`$n` **and**, tolerated,
//!   traditional glyphs `!!`/`!`/`!?`/`?!`/`?`/`??` glued to the SAN),
//!   comments `{...}` — via a small recursive parser, see
//!   [`parse_mainline`]/[`parse_variation`]. Only the main line
//!   (`path`) is replayed via [`GameState::play`] (position/result of the
//!   game affected); any variation is inserted directly into the tree
//!   without ever touching `path` nor the "real" position of the game.

use crate::{
    game::GameState,
    game_tree::{GameTree, Nag},
    history::MoveRecord,
    types::{chess_move::Move, game_state::GameResult, position::Position},
};
use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// PGN Tags
// ---------------------------------------------------------------------------

/// Metadata of a PGN game (Seven Tag Roster).
#[derive(Debug, Clone)]
pub struct PgnTags {
    pub event:  String,
    pub site:   String,
    pub date:   String,
    pub round:  String,
    pub white:  String,
    pub black:  String,
    pub result: String,
}

impl Default for PgnTags {
    fn default() -> Self {
        Self {
            event:  "?".into(),
            site:   "?".into(),
            date:   "????.??.??".into(),
            round:  "?".into(),
            white:  "?".into(),
            black:  "?".into(),
            result: "*".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// PGN Export
// ---------------------------------------------------------------------------

/// Generates a PGN string from a `GameState`.
///
/// Tags are provided as an optional parameter; if `None`, default
/// values are used. The game's result overwrites the one from
/// `GameState`.
#[must_use]
pub fn export_pgn(game: &GameState, tags: Option<PgnTags>) -> String {
    let mut t = tags.unwrap_or_default();

    // Result from the GameState
    match game.result {
        GameResult::WhiteWins => "1-0",
        GameResult::BlackWins => "0-1",
        GameResult::Draw      => "1/2-1/2",
        GameResult::Ongoing   => "*",
    }
    .clone_into(&mut t.result);

    let mut pgn = String::new();

    // --- Seven Tag Roster ---
    // writeln! on a String cannot fail (cf. std impl); the Result
    // is intentionally ignored (clippy::format_push_string).
    let _ = writeln!(pgn, "[Event \"{}\"]", t.event);
    let _ = writeln!(pgn, "[Site \"{}\"]",  t.site);
    let _ = writeln!(pgn, "[Date \"{}\"]",  t.date);
    let _ = writeln!(pgn, "[Round \"{}\"]", t.round);
    let _ = writeln!(pgn, "[White \"{}\"]", t.white);
    let _ = writeln!(pgn, "[Black \"{}\"]", t.black);
    let _ = writeln!(pgn, "[Result \"{}\"]", t.result);
    pgn.push('\n');

    // --- Move sequence (PHASE 16, Step 7.1) ---
    //
    // Walks the entire `GameTree` (main line AND variations, at any
    // depth), not just `history.records()` (which only reflects
    // the active line `path`, cf. `crate::history::History`) —
    // see the doc of `write_siblings`.
    let mut tokens: Vec<String> = Vec::new();
    let tree = game.history().tree();
    write_siblings(tree, tree.roots(), true, &mut tokens);
    tokens.push(t.result.clone());

    // Formatting: 80 columns max
    let mut line = String::new();
    for token in &tokens {
        if line.is_empty() {
            line.push_str(token);
        } else if line.len() + 1 + token.len() > 80 {
            pgn.push_str(&line);
            pgn.push('\n');
            line.clear();
            line.push_str(token);
        } else {
            line.push(' ');
            line.push_str(token);
        }
    }
    if !line.is_empty() {
        pgn.push_str(&line);
        pgn.push('\n');
    }

    pgn
}

// ---------------------------------------------------------------------------
// PGN Export — recursive traversal of the GameTree (PHASE 16, Step 7.1)
// ---------------------------------------------------------------------------

/// Writes into `tokens` the PGN representation of a list of sibling
/// nodes (`siblings`, [`GameTree`] convention: `siblings[0]` = main
/// line from this point, `siblings[1..]` = variations), then continues
/// recursively with the children of the main node — one token per
/// piece of text (move number, SAN, `$n`, `{comment}`), never a single
/// concatenated string, so that the 80-column formatting of
/// [`export_pgn`] can break the line between two tokens without having
/// to reparse the produced text.
///
/// Called both for the entire game (`siblings = tree.roots()`,
/// `force_number = true`) and for each variation encountered
/// (`siblings = &[start_node_id]`, `force_number = true`): a variation
/// is never anything but a single-element list of "siblings" followed
/// by its own continuation, which lets this single function handle any
/// number of nesting levels with no code dedicated per depth —
/// including the special case of an alternative game starting from
/// the very first move (`roots()[1..]`, PHASE 16 decision 1), which is
/// here merely a case of `siblings[1..]` like any other.
///
/// `force_number` indicates whether the move number must be displayed
/// again even for a black move (`"12..."` rather than just the SAN):
/// needed for the very first move of a line (game or variation) and
/// immediately after any interruption (comment or variation inserted
/// before this move) — PGN standard, imposed to remain readable by any
/// compliant reader after a `{...}` or a `(...)`.
fn write_siblings(tree: &GameTree, siblings: &[usize], force_number: bool, tokens: &mut Vec<String>) {
    let Some((&mainline, variations)) = siblings.split_first() else { return };

    let had_comment = write_own_move(tree, mainline, force_number, tokens);

    // A variation inserted after this move interrupts the line just
    // like a comment does: the next move of this line must therefore
    // also display its number again (see doc above).
    let mut interrupted = had_comment;
    for &variation in variations {
        interrupted = true;
        let start = tokens.len();
        // A variation = a single-element list of "siblings": the
        // recursion handles its own continuation and any
        // sub-variations, at any depth, by itself.
        write_siblings(tree, std::slice::from_ref(&variation), true, tokens);
        wrap_in_parens(tokens, start);
    }

    if let Some(node) = tree.node(mainline) {
        write_siblings(tree, &node.children, interrupted, tokens);
    }
}

/// Pushes into `tokens` the tokens of the single move `node_id` (move
/// number if needed, SAN, `$n` if annotated, `{comment}` if present) —
/// building block of [`write_siblings`]. Returns `true` if a comment
/// was pushed (signals to the caller that the line is interrupted, see
/// doc of [`write_siblings`]).
///
/// The move number ("N." for white, always; "N..." for black, only if
/// `force_number`) and the SAN are pushed as two separate tokens — not
/// concatenated — so that [`export_pgn`] separates them with a space
/// during final formatting (convention already in use before this
/// step, e.g. `"1. e4"`, see `test_export_move_numbering`).
///
/// The NAG is exported as a standard numeric token `$n` (PGN Export
/// Format, Edwards 1994, §8.2.3.10), not as a glyph `!!`/`!`/etc.
/// glued to the SAN (legal only in "Import Format", not guaranteed to
/// be supported by all third-party readers) — the glyph remains used
/// as-is on the GUI display side (decision 7,
/// [`GameTree::push_move_text`]), this choice only concerns the
/// `.pgn` file written to disk.
fn write_own_move(tree: &GameTree, node_id: usize, force_number: bool, tokens: &mut Vec<String>) -> bool {
    let Some(node) = tree.node(node_id) else { return false };
    let Some(ply) = tree.ply_index(node_id) else { return false };
    let move_number = ply / 2 + 1;
    let is_white = ply.is_multiple_of(2);

    if is_white {
        tokens.push(format!("{move_number}."));
    } else if force_number {
        tokens.push(format!("{move_number}..."));
    }
    tokens.push(node.record.san.clone());

    if let Some(nag) = node.nag {
        tokens.push(format!("${}", nag.code()));
    }

    if let Some(comment) = &node.comment {
        // Known and accepted (PHASE 16, Step 7): a comment that itself
        // contains `}` is not escaped — would truncate reading at the
        // first `}` encountered on the import side, rare case in practice.
        tokens.push(format!("{{{comment}}}"));
        return true;
    }
    false
}

/// Wraps in parentheses the tokens of `tokens[start..]` (a variation
/// just written by [`write_siblings`]) — `"("` glued to the very first
/// token, `")"` glued to the very last, with no internal space
/// (standard PGN convention: `"(2. Nc3 Nc6)"`, never `"( 2. Nc3 Nc6 )"`).
/// The internal tokens remain separated by a normal space during the
/// final formatting of [`export_pgn`]; only the edge tokens are
/// modified here.
///
/// Does nothing if `start` is out of bounds (should never happen: a
/// variation always pushes at least one number token + one SAN token).
fn wrap_in_parens(tokens: &mut [String], start: usize) {
    let Some(len) = tokens.len().checked_sub(1) else { return };
    if start > len {
        return;
    }
    tokens[start] = format!("({}", tokens[start]);
    tokens[len] = format!("{})", tokens[len]);
}

// ---------------------------------------------------------------------------
// PGN Import
// ---------------------------------------------------------------------------

/// Error while parsing a PGN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgnError {
    /// A mandatory tag is malformed.
    InvalidTag(String),
    /// A SAN move could not be resolved in the position.
    IllegalMove(String),
    /// Empty PGN or with no moves.
    Empty,
    /// A `(...)` variation is never closed (PHASE 16, Step 7.2).
    UnmatchedParenthesis,
    /// A token other than a move (`San`) appears where the parser
    /// expected either a move or a legitimate end of line (robustness
    /// audit 11/07/2026, finding 3.1) — e.g. a `{comment}` or a NAG
    /// glued directly after a move number with no move in between, or a
    /// stray `)` with no matching `(`. Before this fix, `parse_mainline`/
    /// `parse_variation` treated ANY non-`San` token here as a silent,
    /// successful end of line: a PGN starting with a comment before its
    /// first move (`"{Annotated game} 1. e4 e5 *"`, a common Lichess/
    /// engine-analysis export shape) silently imported as a 0-move game
    /// with no error at all.
    UnexpectedToken(String),
}

impl std::fmt::Display for PgnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTag(s)  => write!(f, "Tag PGN invalide : {s}"),
            Self::IllegalMove(s) => write!(f, "Coup SAN illégal : {s}"),
            Self::Empty          => write!(f, "PGN vide"),
            Self::UnmatchedParenthesis => write!(f, "Parenthèse de variante non refermée"),
            Self::UnexpectedToken(s) => write!(f, "Jeton PGN inattendu à la place d'un coup : {s}"),
        }
    }
}

/// Parses a PGN string and returns a `GameState`.
///
/// # Errors
///
/// Returns [`PgnError`] if a move is illegal, if a variation
/// parenthesis is never closed, or if the PGN is empty.
pub fn import_pgn(pgn: &str) -> Result<GameState, PgnError> {
    let tokens = tokenize(pgn);
    if tokens.is_empty() {
        return Err(PgnError::Empty);
    }

    let mut game = GameState::new();
    let mut iter: Tokens<'_> = tokens.iter().peekable();
    parse_mainline(&mut iter, &mut game)?;

    Ok(game)
}

// ---------------------------------------------------------------------------
// PGN Import — recursive parser (PHASE 16, Step 7.2)
// ---------------------------------------------------------------------------

/// A typed token of the PGN stream — replaces the old opaque
/// `Vec<String>`: the recursive parser below needs to distinguish a
/// move from a NAG, a comment, or a parenthesis in order to faithfully
/// rebuild the tree, which a plain string did not allow doing cleanly.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// "1." or "1..." — purely informative, never used to build
    /// the tree (the move number is deduced from the depth in the
    /// tree, see `GameTree::ply_index`): just consumed and ignored.
    MoveNumber,
    /// A move in SAN notation, already cleaned of any annotation
    /// suffix (`!`/`?`) — see `split_annotation_suffix`.
    San(String),
    /// A NAG, whether it comes from a standard `$n` token or from a
    /// traditional suffix (`!!`/`!`/`!?`/`?!`/`?`/`??`) glued to the
    /// previous move — both forms are unified here so the parser has
    /// only one case to handle (see module doc).
    Nag(u8),
    /// Text of a `{...}` comment (braces already stripped).
    Comment(String),
    OpenParen,
    CloseParen,
    /// Game result (`1-0`, `0-1`, `1/2-1/2`, `*`) — never consumed
    /// by the parser (the actual result comes from `GameState::play`,
    /// not from the source text), just an end-of-line marker.
    Result(String),
}

type Tokens<'a> = std::iter::Peekable<std::slice::Iter<'a, Token>>;

// ---------------------------------------------------------------------------
// Tokenization
// ---------------------------------------------------------------------------

/// Splits the PGN into typed tokens (tags ignored; comments, NAGs and
/// variation parentheses captured as full-fledged tokens — see
/// [`Token`] — rather than discarded as before Step 7.2).
fn tokenize(pgn: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = pgn.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            // Tag [...] — always ignored, never useful for
            // rebuilding the tree.
            '[' => {
                for ch in chars.by_ref() {
                    if ch == ']' { break; }
                }
            }
            // Comment {...} — content captured (Step 7.2), then discarded.
            '{' => {
                chars.next();
                let mut text = String::new();
                for ch in chars.by_ref() {
                    if ch == '}' { break; }
                    text.push(ch);
                }
                tokens.push(Token::Comment(text));
            }
            // Variation parentheses: simple edge tokens — it is the
            // recursive parser's job (`parse_variation`), not the
            // tokenizer's, to handle nesting (unlike the old tokenizer
            // which skipped all the content via a depth count).
            '(' => { chars.next(); tokens.push(Token::OpenParen); }
            ')' => { chars.next(); tokens.push(Token::CloseParen); }
            // NAG ($n) — numeric value captured (Step 7.2), then discarded.
            '$' => {
                chars.next();
                let mut digits = String::new();
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    digits.push(chars.next().expect("peek confirmé Some ci-dessus"));
                }
                // `$` with no digit, or code outside 0..=255: token silently
                // dropped (malformed input, out of standard, rare case).
                if let Ok(code) = digits.parse::<u8>() {
                    tokens.push(Token::Nag(code));
                }
            }
            // Spaces / newlines
            c if c.is_whitespace() => { chars.next(); }
            // Token (move, number, result)
            _ => {
                let mut tok = String::new();
                while let Some(&c) = chars.peek() {
                    // `)` added at this step (absent before Step 7.2,
                    // pointless while parentheses were skipped as a
                    // block): a move can be immediately followed by a
                    // closing parenthesis with no space, e.g. the Step
                    // 7.1 export literally produces `"(1... d4)"`.
                    if c.is_whitespace() || matches!(c, '[' | '{' | '(' | ')' | '$') {
                        break;
                    }
                    tok.push(c);
                    chars.next();
                }

                if is_move_number(&tok) {
                    tokens.push(Token::MoveNumber);
                } else if is_result(&tok) {
                    tokens.push(Token::Result(tok));
                } else {
                    // Strip annotations at the end of the token (!?, ?!, !!, ??, !, ?)
                    // and convert them to a NAG rather than discarding them.
                    let (base, suffix) = split_annotation_suffix(&tok);
                    if !base.is_empty() {
                        tokens.push(Token::San(base.to_owned()));
                        if let Some(code) = suffix_to_nag_code(suffix) {
                            tokens.push(Token::Nag(code));
                        }
                    }
                }
            }
        }
    }
    tokens
}

/// Splits a raw token into `(base, suffix)` where `suffix` is the
/// longest trailing run of `!`/`?` (can be empty).
fn split_annotation_suffix(tok: &str) -> (&str, &str) {
    let base = tok.trim_end_matches(['!', '?']);
    (base, &tok[base.len()..])
}

/// Converts a traditional suffix (`!!`, `!`, `!?`, `?!`, `?`, `??`)
/// into a standard NAG code — same table as [`Nag::code`], the other
/// way around. `None` for any other combination (none, or unrecognized
/// — e.g. `!!!`): tolerated silently, not an import error (non-standard
/// glyph ignored, rare case in practice).
fn suffix_to_nag_code(suffix: &str) -> Option<u8> {
    match suffix {
        "!!" => Some(3),
        "!"  => Some(1),
        "!?" => Some(5),
        "?!" => Some(6),
        "?"  => Some(2),
        "??" => Some(4),
        _ => None,
    }
}

fn is_move_number(s: &str) -> bool {
    // "1.", "12.", "1...", "12..."
    let trimmed = s.trim_end_matches('.');
    !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit())
}

fn is_result(s: &str) -> bool {
    matches!(s, "1-0" | "0-1" | "1/2-1/2" | "*")
}

// ---------------------------------------------------------------------------
// Recursive parser — GameTree reconstruction (PHASE 16, Step 7.2)
// ---------------------------------------------------------------------------

/// Consumes the `MoveNumber`/`Comment`/`Nag` tokens at the head of the
/// stream, in any order or repetition — purely informative
/// (`MoveNumber`, see doc of [`Token::MoveNumber`]) or not attachable to
/// any move yet (`Comment`/`Nag` found *before* the first move of a line
/// rather than right after one, e.g. a pre-game annotation such as
/// `"{Annotated by Stockfish} 1. e4 e5 *"`, a shape produced by several
/// real-world PGN exporters). Discarded here rather than treated as
/// [`PgnError::UnexpectedToken`] by [`end_of_line_or_error`] — a
/// deliberate choice (robustness audit 11/07/2026, finding 3.1): the
/// tree only stores comments/NAGs attached to a specific move (see
/// [`consume_annotations`], which handles the equally valid case of an
/// annotation right *after* a move), so a leading annotation has nowhere
/// to attach to and is simply dropped, exactly as it silently was
/// before this fix — the behavior change introduced here only concerns
/// tokens that have no legitimate reading at all in this position
/// (a stray `)`, an `OpenParen` not preceded by any move...), which
/// [`end_of_line_or_error`] now reports as errors instead of silently
/// ending the line.
fn skip_leading_noise(tokens: &mut Tokens<'_>) {
    while matches!(tokens.peek(), Some(Token::MoveNumber | Token::Comment(_) | Token::Nag(_))) {
        tokens.next();
    }
}

/// Consumes, if they immediately follow, the `Nag`/`Comment` tokens of
/// a move just inserted (`node_id`) and applies them to the
/// corresponding node — factored out between [`parse_mainline`] and
/// [`parse_variation`], which share this logic even though they write
/// to the tree via different paths (`GameState::play` vs direct
/// insertion).
///
/// Several consecutive `Nag` tokens (malformed/redundant input, e.g.
/// both a traditional glyph **and** `$n` for the same move): only the
/// last is kept, all are consumed — avoids an unconsumed `Nag` token
/// wrongly blocking the calling loop (which only expects a move or an
/// end of line right after). Same tolerance for several consecutive
/// `Comment`s, concatenated with a space rather than the last one
/// overwriting the previous ones (no information loss in this rare
/// case).
// clippy::while_let_loop: the `loop { match ... { _ => break } }` below
// could be a `while let`, but would require a `Some(&&Token::Nag(code))`
// pattern (double reference, see internal comment) — judged less clear
// than an explicit `match` with an isolated dereference; kept as-is
// deliberately.
#[allow(clippy::while_let_loop)]
fn consume_annotations(tokens: &mut Tokens<'_>, tree: &mut GameTree, node_id: usize) {
    loop {
        // `match` rather than `while let Some(&Token::Nag(code)) = ...`: `peek()`
        // on a `Peekable<slice::Iter<Token>>` returns `Option<&&Token>` (double
        // reference) — extracting `code: u8` via an explicit dereference
        // (`*code`) after a pattern with no explicit `&` is unambiguous, rather
        // than relying on both levels of "match ergonomics" at once.
        let code = match tokens.peek() {
            Some(Token::Nag(code)) => *code,
            _ => break,
        };
        tokens.next();
        if let Some(nag) = Nag::from_code(code) {
            if let Some(node) = tree.node_mut(node_id) {
                node.nag = Some(nag);
            }
        }
    }

    let mut comment: Option<String> = None;
    while matches!(tokens.peek(), Some(Token::Comment(_))) {
        let Some(Token::Comment(text)) = tokens.next() else { unreachable!() };
        comment = Some(match comment {
            Some(existing) => format!("{existing} {text}"),
            None => text.clone(),
        });
    }
    if let Some(text) = comment {
        if let Some(node) = tree.node_mut(node_id) {
            node.comment = Some(text);
        }
    }
}

/// Consumes the closing parenthesis expected after a variation — error
/// if absent (parenthesis never closed, malformed PGN).
fn expect_close_paren(tokens: &mut Tokens<'_>) -> Result<(), PgnError> {
    match tokens.next() {
        Some(Token::CloseParen) => Ok(()),
        _ => Err(PgnError::UnmatchedParenthesis),
    }
}

/// Decides what to do when the parser wants the next mainline move but
/// the next token isn't a `San` — called at the top of the loop body of
/// both [`parse_mainline`] and [`parse_variation`] (robustness audit
/// 11/07/2026, finding 3.1). Returns `Ok(true)` for a legitimate end of
/// line, `Ok(false)` if a move follows (the caller should keep going),
/// `Err` otherwise.
///
/// - `None` (end of the token stream) or a `Result` token (`1-0`, `*`...)
///   always signal a legitimate end of line.
/// - Inside a variation (`accept_close_paren == true`), a `CloseParen`
///   is ALSO a legitimate end: the variation's own body is done, and its
///   caller ([`expect_close_paren`]) consumes that same token right
///   after. At the top level of the main line, `accept_close_paren` is
///   `false`: a `CloseParen` there can only be a stray, unmatched
///   closing parenthesis with no corresponding `(` — an error, not a
///   silently accepted end of game.
/// - Any other token in a move's place (`Comment`, `Nag`, `OpenParen`
///   with no move before it...) is always malformed PGN and returns
///   [`PgnError::UnexpectedToken`] — see its doc for why this matters:
///   before this fix, every one of these cases was silently treated as
///   "end of line" instead, with no error raised at all.
fn end_of_line_or_error(tokens: &mut Tokens<'_>, accept_close_paren: bool) -> Result<bool, PgnError> {
    match tokens.peek() {
        Some(Token::San(_)) => Ok(false),
        None | Some(Token::Result(_)) => Ok(true),
        Some(Token::CloseParen) if accept_close_paren => Ok(true),
        Some(other) => Err(PgnError::UnexpectedToken(format!("{other:?}"))),
    }
}

/// Parses the main line of the game (`path`): each move is played via
/// [`GameState::play`] (position, result, history — all the existing,
/// already-tested behavior, unchanged), then each variation
/// encountered right after (`(...)`, alternative to THIS move) is
/// delegated to [`parse_variation`], which inserts directly into the
/// tree without ever calling `play` nor touching the game's real
/// position.
fn parse_mainline(tokens: &mut Tokens<'_>, game: &mut GameState) -> Result<(), PgnError> {
    loop {
        skip_leading_noise(tokens);
        if end_of_line_or_error(tokens, false)? {
            return Ok(());
        }
        let Some(Token::San(text)) = tokens.next() else { unreachable!() };

        let position_before = game.position().clone();
        let mv = resolve_san(&position_before, text)
            .ok_or_else(|| PgnError::IllegalMove(text.clone()))?;
        game.play(mv).map_err(|_| PgnError::IllegalMove(text.clone()))?;

        let node_id = game.history().last_node_id().expect("coup tout juste joué");
        let parent = game.history().tree().node(node_id).and_then(|n| n.parent);

        consume_annotations(tokens, game.history_mut().tree_mut(), node_id);

        while matches!(tokens.peek(), Some(Token::OpenParen)) {
            tokens.next();
            parse_variation(tokens, game.history_mut().tree_mut(), parent, position_before.clone())?;
            expect_close_paren(tokens)?;
        }
    }
}

/// Parses a variation line (or a sub-variation nested at any
/// depth — calls itself recursively): each move is inserted directly
/// into `tree` via `GameTree::add_move` (never via `GameState::play`,
/// which would wrongly modify the game's real position/result), by
/// advancing a cloned local `Position` at each step.
///
/// `parent`/`position` designate the starting point of this line: for
/// the very first variation encountered, this is the parent and
/// position *of the mainline move right before it* (see call in
/// [`parse_mainline`]) — exactly the same attachment point that
/// [`write_siblings`] uses on export (deliberate symmetry between the
/// two conversion directions).
fn parse_variation(
    tokens: &mut Tokens<'_>,
    tree: &mut GameTree,
    mut parent: Option<usize>,
    mut position: Position,
) -> Result<(), PgnError> {
    loop {
        skip_leading_noise(tokens);
        if end_of_line_or_error(tokens, true)? {
            return Ok(());
        }
        let Some(Token::San(text)) = tokens.next() else { unreachable!() };

        let mv = resolve_san(&position, text).ok_or_else(|| PgnError::IllegalMove(text.clone()))?;
        let record = MoveRecord {
            mv,
            san: crate::notation::move_to_san(&position, mv),
            fen_before: position.to_fen(),
            from_book: false,
        };
        let node_id = tree
            .add_move(parent, record)
            .expect("parent connu, déjà présent dans l'arbre à cette étape");

        let position_before_this_move = position.clone();
        position = crate::rules::make_move(&position, mv)
            .map_err(|_| PgnError::IllegalMove(text.clone()))?;

        consume_annotations(tokens, tree, node_id);

        while matches!(tokens.peek(), Some(Token::OpenParen)) {
            tokens.next();
            parse_variation(tokens, tree, parent, position_before_this_move.clone())?;
            expect_close_paren(tokens)?;
        }

        parent = Some(node_id);
    }
}

// ---------------------------------------------------------------------------
// SAN → Move resolution
// ---------------------------------------------------------------------------

/// Resolves a SAN token into a `Move` legal in the given position.
///
/// Handles: normal moves, captures, promotions, castling, with or without suffix.
fn resolve_san(pos: &crate::types::position::Position, san: &str) -> Option<Move> {
    use crate::{
        movegen::generate_legal_moves,
        notation::move_to_san,
        types::chess_move::MoveKind,
    };

    // Strip the +, # suffixes
    let san_clean = san.trim_end_matches(['+', '#']);

    let legal = generate_legal_moves(pos);

    // Direct attempt: generate the SAN of each legal move and compare
    for &m in &legal {
        let generated = move_to_san(pos, m);
        let generated_clean = generated.trim_end_matches(['+', '#']);
        if generated_clean == san_clean {
            return Some(m);
        }
    }

    // Fallback: try parsing as UCI (e2e4, e7e8q…)
    if let Some(m) = Move::from_uci(san_clean) {
        if legal.contains(&m) {
            return Some(m);
        }
        // For promotions, MoveKind may differ
        if let Some(found) = legal.iter().find(|&&lm| {
            lm.from == m.from && lm.to == m.to
                && lm.kind == MoveKind::Promotion
                && lm.promotion == m.promotion
        }) {
            return Some(*found);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// "Trusted" import — lightweight resolution for bulk import
// (perf bugfix 09/07/2026, see SUIVI_PLAN_ACTION.md: import of a large
// external PGN database intolerably slow, user feedback)
// ---------------------------------------------------------------------------

/// A half-move resolved by [`import_pgn_trusted`]: the position right
/// before this move, and the move itself.
#[derive(Debug, Clone)]
pub struct TrustedPly {
    /// Position BEFORE this move.
    pub position_before: Position,
    /// The resolved move.
    pub mv: Move,
}

/// Result of [`import_pgn_trusted`] — lightweight equivalent of
/// `GameState` for a bulk import from an already-validated source:
/// neither SAN regenerated, nor variation tree, nor
/// checkmate/stalemate detection — only what is needed for the
/// opening-tree indexing of the reference games database
/// (`crates/db`).
#[derive(Debug, Clone)]
pub struct TrustedReplay {
    /// FEN of the starting position — always the standard initial
    /// position today (same limitation as [`import_pgn`]: neither of
    /// these two functions handles a custom `[FEN]`/`[SetUp]` tag).
    pub initial_fen: String,
    /// Total number of half-moves of the main line — independent of
    /// `max_plies` (counts ALL moves of the game, even those not
    /// resolved/kept in `plies`).
    pub ply_count: usize,
    /// One element per **resolved** half-move, in the order played, up
    /// to `max_plies` (see [`import_pgn_trusted`]) — the following
    /// moves, if any, are counted in `ply_count` but neither resolved
    /// nor kept here: useless for the opening tree, bounded to a fixed
    /// depth.
    pub plies: Vec<TrustedPly>,
}

/// Lightweight variant of [`import_pgn`] for a bulk import from an
/// already-validated source (perf bugfix 09/07/2026). Builds neither
/// SAN, nor a variation tree, nor checkmate/stalemate detection — see
/// [`resolve_san_trusted`] for the detail of the gain: pseudo-legal
/// moves + a single king-safety check per move, instead of the full
/// list of legal moves regenerated several times per move by
/// [`import_pgn`] (`resolve_san` + `move_to_san` called for each
/// candidate legal move + `GameState::play` which regenerates the SAN
/// once more + `rules::make_move` which revalidates legality one last
/// time).
///
/// Only the main line is followed (like
/// [`crate::history::History::records`]) — `(...)` variations are
/// ignored (tracked by their parenthesis nesting depth, never
/// resolved); NAG and comments are simply skipped (kept as-is in the
/// original PGN text by the caller, who never regenerates them).
///
/// `max_plies` bounds the number of moves actually resolved/kept in
/// `TrustedReplay::plies` — beyond that, the remaining moves are
/// counted in `ply_count` but neither resolved nor applied (the
/// opening tree of the reference database is bounded to a fixed
/// depth, no need to resolve the rest of a long game).
///
/// # Errors
///
/// Returns [`PgnError::Empty`] if the PGN contains no token,
/// [`PgnError::UnmatchedParenthesis`] if a variation is never closed,
/// or [`PgnError::IllegalMove`] if a main-line move cannot be resolved
/// unambiguously (source nonetheless corrupted/illegal, or
/// insufficient disambiguation) — same error cases as [`import_pgn`],
/// to remain detectable as an "invalid game" on the caller's side.
pub fn import_pgn_trusted(pgn: &str, max_plies: usize) -> Result<TrustedReplay, PgnError> {
    let tokens = tokenize(pgn);
    if tokens.is_empty() {
        return Err(PgnError::Empty);
    }

    let mut pos = Position::starting();
    let initial_fen = pos.to_fen();

    let mut plies: Vec<TrustedPly> = Vec::with_capacity(max_plies.min(64));
    let mut ply_count: usize = 0;
    let mut depth: i32 = 0;

    for token in &tokens {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth -= 1,
            Token::San(text) if depth == 0 => {
                ply_count += 1;
                if plies.len() < max_plies {
                    let mv = resolve_san_trusted(&pos, text)
                        .ok_or_else(|| PgnError::IllegalMove(text.clone()))?;
                    plies.push(TrustedPly { position_before: pos.clone(), mv });
                    pos = crate::movegen::apply_move(&pos, mv)
                        .ok_or_else(|| PgnError::IllegalMove(text.clone()))?;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(PgnError::UnmatchedParenthesis);
    }

    Ok(TrustedReplay { initial_fen, ply_count, plies })
}

/// Resolves a "trusted" SAN token into a `Move`, for
/// [`import_pgn_trusted`]: directly decomposes the SAN text (piece,
/// disambiguation, destination square, promotion) then looks for a
/// match among the **pseudo-legal** moves of the position — never
/// `generate_legal_moves`, which clones the position and rescans the
/// board for EACH pseudo-legal move just to build a list then compared
/// move by move via SAN regeneration (see [`resolve_san`]). A single
/// king-safety check is done here, on the single move finally
/// retained (see [`verify_king_safety`]).
///
/// Castling is always written `O-O`/`O-O-O` in this codebase (never
/// `0-0`/`0-0-0`, see [`crate::notation::move_to_san`]) — standard PGN
/// norm, followed by external sources.
///
/// Returns `None` if the token cannot be decomposed, if no
/// pseudo-legal move matches, if several match (insufficient
/// disambiguation), or if the move leaves its own king in check —
/// same failure cases as [`resolve_san`].
fn resolve_san_trusted(pos: &Position, san: &str) -> Option<Move> {
    use crate::{
        movegen::generate_pseudo_legal,
        types::{chess_move::MoveKind, piece::PieceKind, square::Square},
    };

    // Strip the +, # suffixes (like `resolve_san` — the `!`/`?` have
    // already been removed by the tokenizer, see doc of `Token::San`).
    let base = san.trim_end_matches(['+', '#']);
    let mover = pos.side_to_move;

    // ── Castling ─────────────────────────────────────────────────────────
    if base == "O-O" || base == "O-O-O" {
        let target_file: u8 = if base == "O-O" { 6 } else { 2 };
        let candidates: Vec<Move> = generate_pseudo_legal(pos)
            .into_iter()
            .filter(|m| m.kind == MoveKind::Castle && m.to.file() == target_file)
            .collect();
        let mv = match candidates.as_slice() {
            [only] => *only,
            _ => return None,
        };
        return verify_king_safety(pos, mv, mover).then_some(mv);
    }

    // ── Promotion suffix (e.g. "e8=Q", "exd8=Q+") ───────────────────────
    let (core_part, promotion): (&str, Option<PieceKind>) = match base.find('=') {
        Some(idx) => {
            let piece_char = base[idx + 1..].chars().next()?;
            let kind = piece_letter_to_kind(piece_char.to_ascii_uppercase())?;
            (&base[..idx], Some(kind))
        }
        None => (base, None),
    };

    // ── Piece (uppercase letter N/B/R/Q/K leading) or pawn (no letter)
    let mut chars = core_part.chars();
    let first = chars.next()?;
    let (piece_kind, rest): (PieceKind, &str) = match piece_letter_to_kind(first) {
        Some(pk) => (pk, &core_part[first.len_utf8()..]),
        None => (PieceKind::Pawn, core_part),
    };

    // ── Destination square + disambiguation — the capture 'x' is ignored
    // (it never changes the resolution, only the destination square matters).
    // Going through `Vec<char>` rather than byte-slicing: avoids any
    // risk of panicking on corrupted non-ASCII input (external data,
    // never guaranteed clean despite the "trust" granted to move
    // legality).
    let rest_chars: Vec<char> = rest.chars().filter(|&c| c != 'x').collect();
    if rest_chars.len() < 2 {
        return None;
    }
    let dest_str: String = rest_chars[rest_chars.len() - 2..].iter().collect();
    let dest = Square::from_algebraic(&dest_str)?;

    let mut file_hint: Option<u8> = None;
    let mut rank_hint: Option<u8> = None;
    for &c in &rest_chars[..rest_chars.len() - 2] {
        match c {
            'a'..='h' => file_hint = Some(c as u8 - b'a'),
            '1'..='8' => rank_hint = Some(c as u8 - b'1'),
            _ => return None,
        }
    }

    let candidates: Vec<Move> = generate_pseudo_legal(pos)
        .into_iter()
        .filter(|m| m.to == dest)
        .filter(|m| m.promotion == promotion)
        .filter(|m| pos.board.piece_at(m.from).is_some_and(|p| p.kind == piece_kind))
        .filter(|m| file_hint.is_none_or(|f| m.from.file() == f))
        .filter(|m| rank_hint.is_none_or(|r| m.from.rank() == r))
        .collect();

    let mv = match candidates.as_slice() {
        [only] => *only,
        _ => return None,
    };

    verify_king_safety(pos, mv, mover).then_some(mv)
}

/// Converts a SAN piece letter (uppercase only — `N`/`B`/`R`/
/// `Q`/`K`) into [`crate::types::piece::PieceKind`]. Deliberately
/// **case-sensitive**, unlike `PieceKind::from_fen_char`: SAN
/// distinguishes `B` (Bishop) from `b` (b-file) only by case —
/// using the case-insensitive version here would confuse for example
/// "bxc3" (a pawn from the b-file captures on c3) with a Bishop move.
fn piece_letter_to_kind(c: char) -> Option<crate::types::piece::PieceKind> {
    use crate::types::piece::PieceKind;
    match c {
        'N' => Some(PieceKind::Knight),
        'B' => Some(PieceKind::Bishop),
        'R' => Some(PieceKind::Rook),
        'Q' => Some(PieceKind::Queen),
        'K' => Some(PieceKind::King),
        _ => None,
    }
}

/// Checks that `mv`, played from `pos` by `mover`, does not leave
/// `mover`'s king in check — the only legality check kept by
/// [`resolve_san_trusted`] (unlike `generate_legal_moves`, which
/// applies this same check to EVERY pseudo-legal move to build a full
/// list; here it is done only once, on the single move already
/// identified by text matching).
fn verify_king_safety(pos: &Position, mv: Move, mover: crate::types::piece::Color) -> bool {
    use crate::movegen::{apply_move, is_square_attacked};

    let Some(new_pos) = apply_move(pos, mv) else { return false };
    let Some(king_sq) = new_pos.board.find_king(mover) else { return false };
    !is_square_attacked(&new_pos.board, king_sq, mover.opposite())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::game_state::GameResult;
    // Additional types for the `import_pgn_trusted`/
    // `resolve_san_trusted` tests (perf bugfix 09/07/2026) below.
    use crate::types::{chess_move::MoveKind, square::Square};

    // -----------------------------------------------------------------------
    // Export
    // -----------------------------------------------------------------------

    #[test]
    fn test_export_empty_game() {
        let g = GameState::new();
        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("[Result \"*\"]"));
        assert!(pgn.contains('*'));
    }

    #[test]
    fn test_export_contains_seven_tags() {
        let g = GameState::new();
        let pgn = export_pgn(&g, None);
        for tag in &["Event", "Site", "Date", "Round", "White", "Black", "Result"] {
            assert!(pgn.contains(tag), "Tag manquant: {tag}");
        }
    }

    #[test]
    fn test_export_custom_tags() {
        let g = GameState::new();
        let tags = PgnTags {
            event: "Test Tournament".into(),
            white: "Alice".into(),
            black: "Bob".into(),
            ..PgnTags::default()
        };
        let pgn = export_pgn(&g, Some(tags));
        assert!(pgn.contains("[Event \"Test Tournament\"]"));
        assert!(pgn.contains("[White \"Alice\"]"));
        assert!(pgn.contains("[Black \"Bob\"]"));
    }

    #[test]
    fn test_export_move_numbering() {
        let mut g = GameState::new();
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("e2").unwrap(),
            crate::types::square::Square::from_algebraic("e4").unwrap(),
        )).unwrap();
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("e7").unwrap(),
            crate::types::square::Square::from_algebraic("e5").unwrap(),
        )).unwrap();
        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 e5"));
    }

    #[test]
    fn test_export_result_white_wins() {
        let mut g = GameState::new();
        // Scholar's mate: 1.e4 e5 2.Bc4 Nc6 3.Qh5 Nf6 4.Qxf7#
        let moves = [
            ("e2","e4"), ("e7","e5"),
            ("f1","c4"), ("b8","c6"),
            ("d1","h5"), ("g8","f6"),
            ("h5","f7"),
        ];
        for (f, t) in moves {
            g.play(crate::types::chess_move::Move::normal(
                crate::types::square::Square::from_algebraic(f).unwrap(),
                crate::types::square::Square::from_algebraic(t).unwrap(),
            )).unwrap();
        }
        assert_eq!(g.result, GameResult::WhiteWins);
        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("[Result \"1-0\"]"));
        assert!(pgn.ends_with("1-0\n"));
    }

    // -----------------------------------------------------------------------
    // Export — RAV / NAG / comments (PHASE 16, Step 7.1)
    // -----------------------------------------------------------------------

    fn e4(g: &mut GameState) {
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("e2").unwrap(),
            crate::types::square::Square::from_algebraic("e4").unwrap(),
        )).unwrap();
    }

    fn e5(g: &mut GameState) {
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("e7").unwrap(),
            crate::types::square::Square::from_algebraic("e5").unwrap(),
        )).unwrap();
    }

    fn fake_record(mv_uci: &str, san: &str) -> crate::history::MoveRecord {
        crate::history::MoveRecord {
            mv: Move::from_uci(mv_uci).unwrap(),
            san: san.into(),
            fen_before: String::new(),
            from_book: false,
        }
    }

    #[test]
    fn test_export_nag_as_numeric_dollar_token() {
        let mut g = GameState::new();
        e4(&mut g);
        let id = g.history().last_node_id().unwrap();
        g.history_mut().tree_mut().node_mut(id).unwrap().nag = Some(crate::game_tree::Nag::Brilliant);

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 $3"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_nag_alone_does_not_force_renumbering() {
        let mut g = GameState::new();
        e4(&mut g);
        e5(&mut g);
        let id = g.history().node_id_at(0).unwrap();
        g.history_mut().tree_mut().node_mut(id).unwrap().nag = Some(crate::game_tree::Nag::Brilliant);

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 $3 e5"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_comment_wrapped_in_braces() {
        let mut g = GameState::new();
        e4(&mut g);
        let id = g.history().last_node_id().unwrap();
        g.history_mut().tree_mut().node_mut(id).unwrap().comment = Some("Meilleur coup".into());

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 {Meilleur coup}"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_comment_forces_renumbering_of_next_black_move() {
        let mut g = GameState::new();
        e4(&mut g);
        e5(&mut g);
        let id = g.history().node_id_at(0).unwrap();
        g.history_mut().tree_mut().node_mut(id).unwrap().comment = Some("texte".into());

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 {texte} 1... e5"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_single_variation_as_rav() {
        // 1.e4 e5 (1...d4)
        let mut g = GameState::new();
        e4(&mut g);
        e5(&mut g);
        let e4_id = g.history().node_id_at(0).unwrap();
        g.history_mut().tree_mut().add_move(Some(e4_id), fake_record("d2d4", "d4")).unwrap();

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 e5 (1... d4)"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_root_level_variation_as_rav() {
        // 1.e4 (1.d4): alternative starting from the very first move of the game.
        let mut g = GameState::new();
        e4(&mut g);
        g.history_mut().tree_mut().add_move(None, fake_record("d2d4", "d4")).unwrap();

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("1. e4 (1. d4)"), "PGN produit : {pgn}");
    }

    #[test]
    fn test_export_nested_variation_stays_fully_recursive_not_folded() {
        // 1.e4 e5 2.Nf3 (2.Nc3 Nc6 3.Bc4) 2...Nc6 3.Bb5
        let mut g = GameState::new();
        e4(&mut g);
        e5(&mut g);
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("g1").unwrap(),
            crate::types::square::Square::from_algebraic("f3").unwrap(),
        )).unwrap();
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("b8").unwrap(),
            crate::types::square::Square::from_algebraic("c6").unwrap(),
        )).unwrap();
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("f1").unwrap(),
            crate::types::square::Square::from_algebraic("b5").unwrap(),
        )).unwrap();

        let e5_id = g.history().node_id_at(1).unwrap();
        let nc3_id = g.history_mut().tree_mut().add_move(Some(e5_id), fake_record("b1c3", "Nc3")).unwrap();
        let nc6_var_id = g.history_mut().tree_mut().add_move(Some(nc3_id), fake_record("b8c6", "Nc6")).unwrap();
        g.history_mut().tree_mut().add_move(Some(nc6_var_id), fake_record("f1c4", "Bc4")).unwrap();

        let pgn = export_pgn(&g, None);
        assert!(
            pgn.contains("1. e4 e5 2. Nf3 (2. Nc3 Nc6 3. Bc4) 2... Nc6 3. Bb5"),
            "PGN produit : {pgn}"
        );
    }

    // -----------------------------------------------------------------------
    // Import
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_empty_pgn() {
        let result = import_pgn("   ");
        assert_eq!(result.unwrap_err(), PgnError::Empty);
    }

    #[test]
    fn test_import_single_move() {
        let pgn = "1. e4 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 1);
    }

    #[test]
    fn test_import_two_moves() {
        let pgn = "1. e4 e5 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2);
    }

    #[test]
    fn test_import_illegal_move() {
        let pgn = "1. e5 *"; // illegal from the initial position
        let err = import_pgn(pgn).unwrap_err();
        assert_eq!(err, PgnError::IllegalMove("e5".into()));
    }

    #[test]
    fn test_import_leading_comment_before_first_move_is_skipped_not_truncated() {
        // Robustness audit 11/07/2026, finding 3.1: a comment BEFORE the
        // first move (common shape of real-world PGN exports, e.g.
        // Lichess/engine annotations: "{Annotated game} 1. e4 e5 *") used
        // to silently end `parse_mainline` on its very first loop
        // iteration, producing a 0-move game with no error at all. It
        // must now be skipped (not attachable to any move, see
        // `skip_leading_noise`'s doc) while the moves that follow are
        // still imported normally.
        let pgn = "{Annotated game} 1. e4 e5 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2);
    }

    #[test]
    fn test_import_stray_closing_paren_is_now_an_error() {
        // Robustness audit 11/07/2026, finding 3.1: before this fix, any
        // token other than `San` where a move was expected — including a
        // stray `)` with no matching `(` — silently ended the mainline as
        // if the game were simply over, truncating the rest of the PGN
        // with no error raised. `end_of_line_or_error` now rejects this.
        let pgn = "1. e4 ) e5 *";
        let err = import_pgn(pgn).unwrap_err();
        assert!(
            matches!(err, PgnError::UnexpectedToken(_)),
            "erreur obtenue : {err:?}"
        );
    }

    #[test]
    fn test_import_reconstructs_comment_on_mainline_move() {
        // PHASE 16, Step 7.2: the comment is no longer discarded, it is
        // attached to the corresponding tree node (renamed from
        // `test_import_ignores_comments`, which had become inaccurate).
        let pgn = "1. e4 {bon coup} e5 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2, "la ligne active n'est pas affectée par le commentaire");

        let e4_id = g.history().node_id_at(0).unwrap();
        assert_eq!(g.history().tree().node(e4_id).unwrap().comment.as_deref(), Some("bon coup"));
    }

    #[test]
    fn test_import_reconstructs_nag_from_dollar_token() {
        // Renamed from `test_import_ignores_nags`, which had become inaccurate.
        let pgn = "1. e4$1 e5$2 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2);

        let e4_id = g.history().node_id_at(0).unwrap();
        let e5_id = g.history().node_id_at(1).unwrap();
        assert_eq!(g.history().tree().node(e4_id).unwrap().nag, Some(crate::game_tree::Nag::Good));
        assert_eq!(g.history().tree().node(e5_id).unwrap().nag, Some(crate::game_tree::Nag::Mistake));
    }

    #[test]
    fn test_import_reconstructs_nag_from_suffix_annotation() {
        // Renamed from `test_import_ignores_annotations`, which had become
        // inaccurate: the traditional `!`/`?` glyph is now also
        // recognized as a NAG (tolerance settled Step 7.2), not just `$n`.
        let pgn = "1. e4! e5? *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2);

        let e4_id = g.history().node_id_at(0).unwrap();
        let e5_id = g.history().node_id_at(1).unwrap();
        assert_eq!(g.history().tree().node(e4_id).unwrap().nag, Some(crate::game_tree::Nag::Good));
        assert_eq!(g.history().tree().node(e5_id).unwrap().nag, Some(crate::game_tree::Nag::Mistake));
    }

    #[test]
    fn test_import_reconstructs_root_level_variation() {
        // Renamed from `test_import_ignores_variations`, which had become
        // inaccurate: the variation is now reconstructed in the tree (as
        // a second `roots()`), without ever joining the active line.
        let pgn = "1. e4 (1. d4 d5) e5 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2, "la ligne active reste e4/e5, pas d4/d5");

        let roots = g.history().tree().roots().to_vec();
        assert_eq!(roots.len(), 2, "e4 (mainline) + d4 (variante dès le 1er coup)");
        let d4_id = roots[1];
        assert_eq!(g.history().tree().node(d4_id).unwrap().record.san, "d4");
        let d5_id = g.history().tree().node(d4_id).unwrap().children[0];
        assert_eq!(g.history().tree().node(d5_id).unwrap().record.san, "d5");
    }

    #[test]
    fn test_import_ignores_tags() {
        let pgn = "[Event \"Test\"]\n[White \"Alice\"]\n\n1. e4 e5 *";
        let g = import_pgn(pgn).unwrap();
        assert_eq!(g.move_count(), 2);
    }

    #[test]
    // Clippy (04/07/2026): `#[allow(similar_names)]` — see justification
    // on `test_roundtrip_variation_nag_comment_preserved` (names modeled
    // on the chess notation of the moves under test).
    #[allow(clippy::similar_names)]
    fn test_import_nested_variation_reconstructs_all_levels() {
        // 1.e4 e5 2.Nf3 (2.Nc3 Nc6 (2...Nf6)) Nc6 3.Bb5 — variation of a
        // variation (depth 2), never tested before Step 7.2 (the old
        // parser discarded all the content between parentheses).
        let pgn = "1. e4 e5 2. Nf3 (2. Nc3 Nc6 (2... Nf6)) Nc6 3. Bb5 *";
        let g = import_pgn(pgn).unwrap();

        assert_eq!(g.move_count(), 5, "ligne active : e4 e5 Nf3 Nc6 Bb5");
        let sans: Vec<&str> = g.history().san_list().collect();
        assert_eq!(sans, ["e4", "e5", "Nf3", "Nc6", "Bb5"]);

        // The variation (2.Nc3 Nc6 (2...Nf6)) is a child of e5, alternative to Nf3.
        let e5_id = g.history().node_id_at(1).unwrap();
        let nc3_id = g.history().tree().node(e5_id).unwrap().children[1];
        assert_eq!(g.history().tree().node(nc3_id).unwrap().record.san, "Nc3");

        let nc6_var_id = g.history().tree().node(nc3_id).unwrap().children[0];
        assert_eq!(g.history().tree().node(nc6_var_id).unwrap().record.san, "Nc6");

        // The sub-variation (2...Nf6) is a child of Nc3 (alternative to Nc6).
        let nf6_id = g.history().tree().node(nc3_id).unwrap().children[1];
        assert_eq!(g.history().tree().node(nf6_id).unwrap().record.san, "Nf6");
    }

    #[test]
    fn test_import_unmatched_parenthesis_returns_error() {
        let pgn = "1. e4 (1. d4 *";
        let err = import_pgn(pgn).unwrap_err();
        assert_eq!(err, PgnError::UnmatchedParenthesis);
    }

    #[test]
    fn test_import_illegal_move_inside_variation_is_an_error() {
        let pgn = "1. e4 (1. e5) *"; // e5 illegal from the initial position
        let err = import_pgn(pgn).unwrap_err();
        assert_eq!(err, PgnError::IllegalMove("e5".into()));
    }

    #[test]
    fn test_import_variation_does_not_affect_mainline_result_or_position() {
        // The variation contains a move that would lead to a very
        // different position (Qxf7 as early as the 2nd move) — the game's
        // real result/position must only reflect the active line.
        let pgn = "1. e4 e5 2. Nf3 (2. Qh5 Nc6 3. Qxf7) Nc6 *";
        let g = import_pgn(pgn).unwrap();

        assert_eq!(g.result, GameResult::Ongoing);
        assert_eq!(g.move_count(), 4);
        let sans: Vec<&str> = g.history().san_list().collect();
        assert_eq!(sans, ["e4", "e5", "Nf3", "Nc6"]);
    }

    // -----------------------------------------------------------------------
    // import_pgn_trusted / resolve_san_trusted (perf bugfix 09/07/2026)
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_pgn_trusted_empty() {
        assert_eq!(import_pgn_trusted("", 64).unwrap_err(), PgnError::Empty);
    }

    #[test]
    fn test_import_pgn_trusted_illegal_move() {
        // e5 is not playable from the initial position.
        let err = import_pgn_trusted("1. e5 *", 64).unwrap_err();
        assert_eq!(err, PgnError::IllegalMove("e5".into()));
    }

    #[test]
    fn test_import_pgn_trusted_unmatched_parenthesis() {
        let err = import_pgn_trusted("1. e4 (1. d4 *", 64).unwrap_err();
        assert_eq!(err, PgnError::UnmatchedParenthesis);
    }

    #[test]
    fn test_import_pgn_trusted_ignores_variations() {
        // Same game as `test_import_variation_does_not_affect_mainline_result_or_position`:
        // only the main line must be resolved and kept.
        let pgn = "1. e4 e5 2. Nf3 (2. Qh5 Nc6 3. Qxf7) Nc6 *";
        let replay = import_pgn_trusted(pgn, 64).unwrap();
        assert_eq!(replay.ply_count, 4);
        let uci: Vec<String> = replay.plies.iter().map(|p| p.mv.to_uci()).collect();
        assert_eq!(uci, ["e2e4", "e7e5", "g1f3", "b8c6"]);
    }

    #[test]
    fn test_import_pgn_trusted_respects_max_plies() {
        // 5 half-moves played, but only the first 3 must be resolved and
        // kept in `plies`; `ply_count` must reflect the real total.
        let pgn = "1. e4 e5 2. Nf3 Nc6 3. Bb5 *";
        let replay = import_pgn_trusted(pgn, 3).unwrap();
        assert_eq!(replay.ply_count, 5);
        assert_eq!(replay.plies.len(), 3);
        let uci: Vec<String> = replay.plies.iter().map(|p| p.mv.to_uci()).collect();
        assert_eq!(uci, ["e2e4", "e7e5", "g1f3"]);
    }

    #[test]
    fn test_import_pgn_trusted_matches_import_pgn_on_scholars_mate() {
        // The final move carries a checkmate suffix ("Qxf7#") — also
        // checks that `resolve_san_trusted` correctly handles this suffix.
        let pgn = "1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6 4. Qxf7# 1-0";

        let g = import_pgn(pgn).unwrap();
        let expected: Vec<String> =
            g.history().records().iter().map(|r| r.mv.to_uci()).collect();

        let replay = import_pgn_trusted(pgn, 64).unwrap();
        let actual: Vec<String> = replay.plies.iter().map(|p| p.mv.to_uci()).collect();

        assert_eq!(actual, expected);
        assert_eq!(actual, ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"]);
    }

    #[test]
    fn test_resolve_san_trusted_disambiguation_by_file() {
        // Knights on b1 and d1, both able to reach c3.
        let pos = Position::from_fen("4k3/8/8/8/8/8/8/1N1NK3 w - - 0 1").unwrap();

        // Without disambiguation: ambiguous, must fail.
        assert!(resolve_san_trusted(&pos, "Nc3").is_none());

        let mv_b = resolve_san_trusted(&pos, "Nbc3").unwrap();
        assert_eq!(mv_b.from, Square::from_algebraic("b1").unwrap());
        assert_eq!(mv_b.to, Square::from_algebraic("c3").unwrap());

        let mv_d = resolve_san_trusted(&pos, "Ndc3").unwrap();
        assert_eq!(mv_d.from, Square::from_algebraic("d1").unwrap());
        assert_eq!(mv_d.to, Square::from_algebraic("c3").unwrap());
    }

    #[test]
    fn test_resolve_san_trusted_disambiguation_by_rank() {
        // Rooks on d1 and d5 (same file) able to reach d3 —
        // rank disambiguation required.
        let pos = Position::from_fen("4k3/8/8/3R4/8/8/8/3RK3 w - - 0 1").unwrap();

        let mv_1 = resolve_san_trusted(&pos, "R1d3").unwrap();
        assert_eq!(mv_1.from, Square::from_algebraic("d1").unwrap());

        let mv_5 = resolve_san_trusted(&pos, "R5d3").unwrap();
        assert_eq!(mv_5.from, Square::from_algebraic("d5").unwrap());
    }

    #[test]
    fn test_resolve_san_trusted_castling_white_both_sides() {
        let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();

        let oo = resolve_san_trusted(&pos, "O-O").unwrap();
        assert_eq!(oo.kind, MoveKind::Castle);
        assert_eq!(oo.from, Square::from_algebraic("e1").unwrap());
        assert_eq!(oo.to, Square::from_algebraic("g1").unwrap());

        let ooo = resolve_san_trusted(&pos, "O-O-O").unwrap();
        assert_eq!(ooo.kind, MoveKind::Castle);
        assert_eq!(ooo.from, Square::from_algebraic("e1").unwrap());
        assert_eq!(ooo.to, Square::from_algebraic("c1").unwrap());
    }

    #[test]
    fn test_resolve_san_trusted_castling_black_both_sides() {
        let pos = Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1").unwrap();

        let oo = resolve_san_trusted(&pos, "O-O").unwrap();
        assert_eq!(oo.kind, MoveKind::Castle);
        assert_eq!(oo.from, Square::from_algebraic("e8").unwrap());
        assert_eq!(oo.to, Square::from_algebraic("g8").unwrap());

        let ooo = resolve_san_trusted(&pos, "O-O-O").unwrap();
        assert_eq!(ooo.kind, MoveKind::Castle);
        assert_eq!(ooo.from, Square::from_algebraic("e8").unwrap());
        assert_eq!(ooo.to, Square::from_algebraic("c8").unwrap());
    }

    #[test]
    fn test_resolve_san_trusted_promotion() {
        let pos = Position::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let mv = resolve_san_trusted(&pos, "a8=Q").unwrap();
        assert_eq!(mv.kind, MoveKind::Promotion);
        assert_eq!(mv.from, Square::from_algebraic("a7").unwrap());
        assert_eq!(mv.to, Square::from_algebraic("a8").unwrap());
        assert_eq!(mv.promotion, Some(crate::types::piece::PieceKind::Queen));
    }

    #[test]
    fn test_resolve_san_trusted_en_passant() {
        // Black pawn having just played d7-d5: the en passant capture
        // square d6 is available for the white pawn on e5.
        let pos = Position::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").unwrap();
        let mv = resolve_san_trusted(&pos, "exd6").unwrap();
        assert_eq!(mv.kind, MoveKind::EnPassant);
        assert_eq!(mv.from, Square::from_algebraic("e5").unwrap());
        assert_eq!(mv.to, Square::from_algebraic("d6").unwrap());
    }

    #[test]
    fn test_resolve_san_trusted_rejects_move_exposing_own_king() {
        // White rook pinned on the e-file (king e1, opposing rook e8):
        // moving it off the file exposes the king, must be rejected
        // despite a pseudo-legal disambiguation and destination.
        let pos = Position::from_fen("4r3/8/8/8/8/8/4R3/4K3 w - - 0 1").unwrap();

        assert!(resolve_san_trusted(&pos, "Rd2").is_none());

        // Staying on the e-file remains safe (the king stays protected).
        let safe = resolve_san_trusted(&pos, "Re3").unwrap();
        assert_eq!(safe.from, Square::from_algebraic("e2").unwrap());
        assert_eq!(safe.to, Square::from_algebraic("e3").unwrap());
    }

    // -----------------------------------------------------------------------
    // Export → import round trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_roundtrip_scholars_mate() {
        let mut g = GameState::new();
        let moves = [
            ("e2","e4"), ("e7","e5"),
            ("f1","c4"), ("b8","c6"),
            ("d1","h5"), ("g8","f6"),
            ("h5","f7"),
        ];
        for (f, t) in moves {
            g.play(crate::types::chess_move::Move::normal(
                crate::types::square::Square::from_algebraic(f).unwrap(),
                crate::types::square::Square::from_algebraic(t).unwrap(),
            )).unwrap();
        }

        let pgn = export_pgn(&g, None);
        let g2  = import_pgn(&pgn).unwrap();

        assert_eq!(g.move_count(),   g2.move_count());
        assert_eq!(g.result,         g2.result);
        assert_eq!(g.position().to_fen(), g2.position().to_fen());

        // Check that the SANs are identical
        let sans1: Vec<&str> = g.history().san_list().collect();
        let sans2: Vec<&str> = g2.history().san_list().collect();
        assert_eq!(sans1, sans2);
    }

    #[test]
    fn test_roundtrip_fools_mate() {
        // 1.f3 e5 2.g4 Qh4#
        let mut g = GameState::new();
        let moves = [
            ("f2","f3"), ("e7","e5"),
            ("g2","g4"), ("d8","h4"),
        ];
        for (f, t) in moves {
            g.play(crate::types::chess_move::Move::normal(
                crate::types::square::Square::from_algebraic(f).unwrap(),
                crate::types::square::Square::from_algebraic(t).unwrap(),
            )).unwrap();
        }
        assert_eq!(g.result, GameResult::BlackWins);

        let pgn = export_pgn(&g, None);
        let g2  = import_pgn(&pgn).unwrap();

        assert_eq!(g.move_count(), g2.move_count());
        assert_eq!(g.result,       g2.result);
        assert_eq!(g.position().to_fen(), g2.position().to_fen());
    }

    #[test]
    fn test_roundtrip_with_castling() {
        // 1.e4 e5 2.Nf3 Nc6 3.Bc4 Bc5 4.O-O Nf6
        let mut g = GameState::new();
        let moves_uci = [
            ("e2","e4"), ("e7","e5"),
            ("g1","f3"), ("b8","c6"),
            ("f1","c4"), ("f8","c5"),
        ];
        for (f, t) in moves_uci {
            g.play(crate::types::chess_move::Move::normal(
                crate::types::square::Square::from_algebraic(f).unwrap(),
                crate::types::square::Square::from_algebraic(t).unwrap(),
            )).unwrap();
        }
        // White castling
        g.play(crate::types::chess_move::Move::castle(
            crate::types::square::Square::from_algebraic("e1").unwrap(),
            crate::types::square::Square::from_algebraic("g1").unwrap(),
        )).unwrap();
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("g8").unwrap(),
            crate::types::square::Square::from_algebraic("f6").unwrap(),
        )).unwrap();

        let pgn = export_pgn(&g, None);
        assert!(pgn.contains("O-O"), "Le roque doit apparaître dans le PGN");

        let g2 = import_pgn(&pgn).unwrap();
        assert_eq!(g.move_count(), g2.move_count());
        assert_eq!(g.position().to_fen(), g2.position().to_fen());
    }

    #[test]
    // Clippy (04/07/2026): `#[allow(similar_names)]` — the variable names
    // (nc3_id, nc6_id, nf3_id2…) deliberately mirror the chess notation
    // of the moves under test (Nc3, Nc6, Nf3), not an accidental mix-up.
    #[allow(clippy::similar_names)]
    fn test_roundtrip_variation_nag_comment_preserved() {
        // End-to-end proof of the Step 7.1 (export) / 7.2 (import)
        // symmetry: NAG + comment on a main-line move, and a variation
        // with its own sub-variation, must all survive an
        // export → import round trip.
        let mut g = GameState::new();
        e4(&mut g);
        e5(&mut g);
        g.play(crate::types::chess_move::Move::normal(
            crate::types::square::Square::from_algebraic("g1").unwrap(),
            crate::types::square::Square::from_algebraic("f3").unwrap(),
        )).unwrap();
        let nf3_id = g.history().last_node_id().unwrap();
        g.history_mut().tree_mut().node_mut(nf3_id).unwrap().nag = Some(crate::game_tree::Nag::Good);
        g.history_mut().tree_mut().node_mut(nf3_id).unwrap().comment = Some("Solide".into());

        let e5_id = g.history().node_id_at(1).unwrap();
        let nc3_id = g.history_mut().tree_mut().add_move(Some(e5_id), fake_record("b1c3", "Nc3")).unwrap();
        let nc6_id = g.history_mut().tree_mut().add_move(Some(nc3_id), fake_record("b8c6", "Nc6")).unwrap();
        g.history_mut().tree_mut().add_move(Some(nc3_id), fake_record("g8f6", "Nf6")).unwrap();

        let pgn = export_pgn(&g, None);
        let g2 = import_pgn(&pgn).unwrap();

        assert_eq!(g2.move_count(), 3, "PGN produit : {pgn}");
        let sans2: Vec<&str> = g2.history().san_list().collect();
        assert_eq!(sans2, ["e4", "e5", "Nf3"]);

        let nf3_id2 = g2.history().node_id_at(2).unwrap();
        assert_eq!(g2.history().tree().node(nf3_id2).unwrap().nag, Some(crate::game_tree::Nag::Good));
        assert_eq!(g2.history().tree().node(nf3_id2).unwrap().comment.as_deref(), Some("Solide"));

        let e5_id2 = g2.history().node_id_at(1).unwrap();
        let nc3_id2 = g2.history().tree().node(e5_id2).unwrap().children[1];
        assert_eq!(g2.history().tree().node(nc3_id2).unwrap().record.san, "Nc3");

        let nc6_id2 = g2.history().tree().node(nc3_id2).unwrap().children[0];
        assert_eq!(g2.history().tree().node(nc6_id2).unwrap().record.san, "Nc6");
        let nf6_id2 = g2.history().tree().node(nc3_id2).unwrap().children[1];
        assert_eq!(g2.history().tree().node(nf6_id2).unwrap().record.san, "Nf6");

        let _ = nc6_id; // used only to build the starting tree
    }
}
