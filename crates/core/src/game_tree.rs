//! Variation tree of a game (PHASE 16 — PGN Variations).
//!
//! [`GameTree`] is a **pure** data structure, tested in isolation at
//! this stage: it is not yet wired into [`crate::game::GameState`]
//! nor [`crate::history::History`] (planned for Step 2, see
//! `Analyse_Projet/SUIVI_PLAN_ACTION.md`, PHASE 16).
//!
//! Each node ([`GameNode`]) represents a move played (reuses
//! [`crate::history::MoveRecord`], no type duplication) and can have
//! several children: `children[0]` is always the "main line"
//! continuation from this node, the following entries are variations —
//! same convention as the PGN standard and Lichess. This convention is
//! maintained by [`GameTree::add_move`] (a new branch is always
//! appended in last position, never replacing an existing
//! branch — decision settled: no silent truncation) and
//! [`GameTree::promote_to_mainline`] (explicitly reorders).

use crate::history::MoveRecord;
use std::collections::HashMap;
use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Nag
// ---------------------------------------------------------------------------

/// NAG (Numeric Annotation Glyph) move-quality annotation.
///
/// Subset of the NAG glyphs from the PGN standard (`$1` to `$6`) exposed in
/// Vendetta's context menu (PHASE 16, decision 7), ordered here by
/// polarity from best to worst to match the menu's display
/// order: [`Nag::Brilliant`], [`Nag::Good`], [`Nag::Interesting`],
/// [`Nag::Dubious`], [`Nag::Mistake`], [`Nag::Blunder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nag {
    /// `!!` — brilliant move (NAG `$3`).
    Brilliant,
    /// `!` — good move (NAG `$1`).
    Good,
    /// `!?` — interesting move (NAG `$5`).
    Interesting,
    /// `?!` — dubious move (NAG `$6`).
    Dubious,
    /// `?` — mistake (NAG `$2`).
    Mistake,
    /// `??` — blunder (NAG `$4`).
    Blunder,
}

impl Nag {
    /// Traditional text symbol displayed next to the annotated move.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Nag::Brilliant => "!!",
            Nag::Good => "!",
            Nag::Interesting => "!?",
            Nag::Dubious => "?!",
            Nag::Mistake => "?",
            Nag::Blunder => "??",
        }
    }

    /// Corresponding standard PGN numeric code (`$1` to `$6`) — used
    /// for PGN export/import (PHASE 16, Step 7, not done yet).
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Nag::Good => 1,
            Nag::Mistake => 2,
            Nag::Brilliant => 3,
            Nag::Blunder => 4,
            Nag::Interesting => 5,
            Nag::Dubious => 6,
        }
    }

    /// Reconstructs a [`Nag`] from its standard PGN numeric code.
    ///
    /// `None` for any code outside `1..=6`: the PGN standard defines other
    /// NAG glyphs (up to `$255`), but only these six are exposed in
    /// Vendetta's context menu (decision settled, PHASE 16).
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Nag::Good),
            2 => Some(Nag::Mistake),
            3 => Some(Nag::Brilliant),
            4 => Some(Nag::Blunder),
            5 => Some(Nag::Interesting),
            6 => Some(Nag::Dubious),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// GameNode
// ---------------------------------------------------------------------------

/// A node of the variation tree: a move played, its possible following
/// moves, and its annotations.
///
/// Fields are public (consistent with [`MoveRecord`], already public in
/// all its fields): modifying `comment`/`nag` directly via
/// [`GameTree::node_mut`] is the expected pattern (same principle as
/// [`crate::history::History::last_mut`]). Modifying `children`/`parent`
/// directly is, however, discouraged — go through
/// [`GameTree::add_move`]/[`GameTree::promote_to_mainline`]/
/// [`GameTree::remove_subtree`] to keep the tree consistent.
#[derive(Debug, Clone)]
pub struct GameNode {
    /// Stable node identifier — never changes, even after
    /// other nodes of the tree are removed.
    pub id: usize,
    /// Parent node, or `None` if this node is a root (first move of a
    /// line — main line at [`GameTree::roots`]`()[0]`, variations
    /// starting from the very first move of the game after that).
    pub parent: Option<usize>,
    /// Following moves from this position. `children[0]` = main
    /// line, following entries = variations.
    pub children: Vec<usize>,
    /// The move played to reach this node.
    pub record: MoveRecord,
    /// Free-text comment attached to this move (inline editing, PHASE 16
    /// decision 8). `None` = no comment.
    pub comment: Option<String>,
    /// NAG quality annotation attached to this move. `None` = no
    /// annotation.
    pub nag: Option<Nag>,
}

// ---------------------------------------------------------------------------
// FlatNode
// ---------------------------------------------------------------------------

/// An entry of a flattened traversal of [`GameTree`] (see [`GameTree::flatten`]),
/// with depth and parentage — building block for displaying variations
/// (PHASE 16, Step 3: structure produced and tested here, not yet
/// consumed by the Slint UI, see Step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlatNode {
    /// Node identifier in the tree.
    pub node_id: usize,
    /// Nesting depth: `0` = main line, `1` = first-level
    /// variation, `2` = variation of a variation, etc.
    pub depth: usize,
    /// Parent node identifier, or `None` if this node is a root.
    pub parent_id: Option<usize>,
    /// `true` if this node is on the main line of the game (equivalent
    /// to [`GameTree::is_mainline`], recomputed here to avoid a second O(depth)
    /// call by the caller).
    pub is_mainline: bool,
}

// ---------------------------------------------------------------------------
// VariationBlock
// ---------------------------------------------------------------------------

/// A variation block ready to display (PHASE 16, Step 4), produced by
/// [`GameTree::build_variation_blocks`].
///
/// A block corresponds to a full depth-1 variation:
/// any nested sub-variation (depth ≥2) is already folded in
/// inline parentheses in `text` (decision 4) — a single visual block per
/// first-level variation, regardless of its internal complexity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariationBlock {
    /// Index (convention [`crate::history::History::get`]) of the main
    /// line move right after which this block must be displayed.
    pub after_ply: usize,
    /// PGN-style formatted text of the entire variation ("N." or
    /// "N..." numbering included, sub-variations folded into `(...)`, any
    /// NAG glyphs included since Step 6.1).
    pub text: String,
    /// Node identifier of the **first move** of this variation (the one
    /// that defines the branch). PHASE 16, Step 6.1: target of a right-click on this
    /// block — a folded block does not expose its internal moves individually
    /// (decision settled), so the context menu acts on this first move
    /// (NAG/comment), or on the entire branch it represents
    /// (promotion/deletion, planned for Step 6.2).
    pub start_node_id: usize,
}

// ---------------------------------------------------------------------------
// FlattenTask (explicit stack for GameTree::flatten_expand)
// ---------------------------------------------------------------------------

/// Pending instruction on the explicit stack of
/// [`GameTree::flatten_expand`] (code audit 04/07/2026, point 1 — replaces
/// a recursion with an equivalent iterative traversal, to never risk
/// stack overflow on an extremely long main line).
enum FlattenTask {
    /// Pushes the record of `node_id` as a variation (depth
    /// `depth`, parent `parent_id`), then schedules the expansion of its
    /// own children.
    Variation { node_id: usize, depth: usize, parent_id: usize },
    /// Expands the children of `node_id` (already present in `out`, on the
    /// main line iff `node_is_mainline`): pushes the record of
    /// its "main line" child (which inherits `node_is_mainline`), then
    /// schedules each variation (in order) and finally the continuation of the
    /// main line.
    Expand { node_id: usize, depth: usize, node_is_mainline: bool },
}

// ---------------------------------------------------------------------------
// RenderTask (explicit stack for GameTree::render_continue)
// ---------------------------------------------------------------------------

/// Pending instruction on the explicit stack of
/// [`GameTree::render_continue`] (robustness audit 11/07/2026, finding
/// 2.2 — same fix as [`FlattenTask`]/`flatten_expand` on 04/07/2026,
/// applied here belatedly since this textual traversal walks the exact
/// same tree shape and was just as exposed to a stack overflow on an
/// extremely long imported PGN main line).
///
/// Both variants below carry the `ply` (see [`GameTree::ply_index`]) of
/// `node_id` itself, computed once by the task that *pushed* them rather
/// than recomputed via `ply_index`'s `O(depth)` ancestor walk every time
/// `node_id` is visited. Follow-up caught while adding this fix's own
/// non-regression test (100,000-move chain): `render_continue` calling
/// `ply_index` — via `push_move_text`, a helper since removed in favour of
/// [`GameTree::push_move_text_at_ply`] — once per node of an
/// `N`-move line costs `O(N)` per call and is itself called `O(N)`
/// times, i.e. `O(N²)` total. This pre-existed the 11/07/2026 stack-safety
/// fix (the *recursive* version had exactly the same per-node cost) but
/// was never actually reachable at a problematic depth before: a
/// 100,000-move recursive call chain overflowed the stack (a crash) long
/// before `O(N²)` could matter. Fixing the stack overflow without also
/// fixing this would have traded an immediate crash for a multi-minute
/// hang instead (`cargo test` observed stuck past 60 s) — exactly the
/// same class of `O(N²)` trap already identified and fixed for
/// `is_mainline`/`flatten_expand` on 04/07/2026 (see
/// [`GameTree::flatten_expand`]'s "Perf note"), same fix applied here:
/// propagate rather than recompute.
enum RenderTask {
    /// Writes `" ("` then the first move token of the variation rooted at
    /// `node_id` (at ply `ply`) into the output buffer, then schedules the
    /// expansion of its own continuation — mirrors [`FlattenTask::Variation`].
    OpenVariation { node_id: usize, ply: usize },
    /// Expands the continuation after `node_id` (itself at ply `ply`):
    /// writes its "main line" child's move text, then schedules each of
    /// `node_id`'s variations (each opened via [`Self::OpenVariation`],
    /// each eventually closed by a matching [`Self::CloseParen`]), and
    /// finally the continuation of the main line itself — mirrors
    /// [`FlattenTask::Expand`].
    Expand { node_id: usize, ply: usize },
    /// Writes the closing `)` of a variation, once its [`Self::Expand`]
    /// task — and everything nested inside it, however deep — has been
    /// fully processed. Has no equivalent in [`FlattenTask`]: it stands in
    /// for the code that used to run right after the recursive call
    /// returned in the pre-11/07/2026 version of `render_continue`, which
    /// an explicit LIFO stack cannot express other than as its own
    /// deferred instruction.
    CloseParen,
}

// ---------------------------------------------------------------------------
// GameTree
// ---------------------------------------------------------------------------

/// Variation tree of a game.
///
/// Internal representation as [`HashMap`]`<usize, GameNode>` rather than a
/// position-indexed `Vec` (unlike
/// [`crate::history::History`]): unlike the existing linear
/// history, this tree must support **removing** a node (and
/// its descendants, see [`Self::remove_subtree`]) without ever invalidating the
/// identifiers of the remaining nodes. A position-indexed `Vec` would have
/// required either shifting the remaining indices (invalidates ids already
/// distributed), or empty slots ("tombstones") to manage
/// manually — the `HashMap` with a strictly increasing identifier counter
/// (`next_id`, never reused) avoids this problem simply.
#[derive(Debug, Clone, Default)]
pub struct GameTree {
    nodes: HashMap<usize, GameNode>,
    roots: Vec<usize>,
    next_id: usize,
}

impl GameTree {
    /// Creates an empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Total number of nodes in the tree (main line and variations
    /// combined).
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` if the tree contains no moves.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Identifiers of the root nodes (moves without a parent): the main
    /// line of the game always starts at `roots()[0]` if the tree
    /// is not empty; the following entries are variations starting
    /// from the very first move of the game.
    #[must_use]
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    /// Reference to a node, or `None` if the identifier is unknown
    /// (never created, or already removed by [`Self::remove_subtree`]).
    #[must_use]
    pub fn node(&self, id: usize) -> Option<&GameNode> {
        self.nodes.get(&id)
    }

    /// Mutable reference to a node — used in particular to modify
    /// `comment`/`nag` directly (same pattern as
    /// [`crate::history::History::last_mut`]).
    pub fn node_mut(&mut self, id: usize) -> Option<&mut GameNode> {
        self.nodes.get_mut(&id)
    }

    /// Adds a move as a new branch from `parent` (or as a
    /// new root if `parent` is `None`), always in **last**
    /// position of the relevant children list — never replaces or
    /// reorders existing branches (PHASE 16, decision 1:
    /// no silent truncation/overwriting of already-recorded moves).
    ///
    /// Returns the new identifier, or `None` if `parent` is `Some(id)`
    /// with an `id` unknown in the tree (no modification made
    /// in that case).
    ///
    /// # Panics
    /// Does not panic in practice: the existence of `parent` in `self.nodes`
    /// is checked right above, and this method does not run in
    /// a concurrent context (`&mut self`) where the node could disappear
    /// between the two lines.
    pub fn add_move(&mut self, parent: Option<usize>, record: MoveRecord) -> Option<usize> {
        if let Some(pid) = parent {
            if !self.nodes.contains_key(&pid) {
                return None;
            }
        }

        let id = self.next_id;
        self.next_id += 1;

        self.nodes.insert(
            id,
            GameNode { id, parent, children: Vec::new(), record, comment: None, nag: None },
        );

        match parent {
            Some(pid) => {
                // Existence already checked above: the parent cannot
                // have disappeared between the two lines (no concurrency).
                self.nodes
                    .get_mut(&pid)
                    .expect("parent vérifié existant juste au-dessus")
                    .children
                    .push(id);
            }
            None => self.roots.push(id),
        }

        Some(id)
    }

    /// Reorders the siblings of node `id` so that it becomes the main
    /// line from its parent — moved to the first position of the
    /// parent's children list (or of [`Self::roots`] if it has no
    /// parent). PHASE 16, decision 2 ("Promote to main line").
    ///
    /// Only affects a single level of the tree (the immediate siblings): if the
    /// node's parent is not itself on the main line, this node
    /// becomes the main line *from its parent*, but the
    /// full path from the root does not automatically become "main
    /// line" as a result (see [`Self::is_mainline`]).
    ///
    /// Returns `false` if `id` is unknown (no modification).
    pub fn promote_to_mainline(&mut self, id: usize) -> bool {
        let Some(node) = self.nodes.get(&id) else {
            return false;
        };
        let parent = node.parent;

        let siblings: &mut Vec<usize> = match parent {
            Some(pid) => match self.nodes.get_mut(&pid) {
                Some(p) => &mut p.children,
                None => return false, // normally impossible inconsistency
            },
            None => &mut self.roots,
        };

        let Some(pos) = siblings.iter().position(|&sid| sid == id) else {
            return false;
        };
        if pos != 0 {
            let sibling = siblings.remove(pos);
            siblings.insert(0, sibling);
        }
        true
    }

    /// Removes node `id` and all its descendants (nested variations
    /// included) — PHASE 16, decision 7 ("Remove this variation").
    ///
    /// Expected side effect: if `id` was in first position (main
    /// line from its parent) and had siblings, the next sibling
    /// naturally becomes the new main line at that
    /// spot (standard list shift) — behavior deemed reasonable
    /// (no "empty main line" after removal).
    ///
    /// Returns `false` if `id` is unknown (no modification).
    pub fn remove_subtree(&mut self, id: usize) -> bool {
        if !self.nodes.contains_key(&id) {
            return false;
        }

        // Detaches `id` from its parent (or from `roots`) before purging the
        // subtree, so as to never leave a dangling reference to a
        // removed node.
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        let siblings: Option<&mut Vec<usize>> = match parent {
            Some(pid) => self.nodes.get_mut(&pid).map(|p| &mut p.children),
            None => Some(&mut self.roots),
        };
        if let Some(siblings) = siblings {
            siblings.retain(|&sid| sid != id);
        }

        // Recursive purge of the subtree via an explicit stack (no
        // Rust recursion): avoids any call-depth limit for
        // a very heavily annotated game.
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if let Some(node) = self.nodes.remove(&current) {
                stack.extend(node.children);
            }
        }

        true
    }

    /// `true` if `id` is on the main line of the game, i.e.
    /// if it and all its ancestors are in the first position of their
    /// sibling list (`roots()[0]` at the root).
    ///
    /// `false` for an unknown identifier (safe default value rather
    /// than panicking).
    ///
    /// Implemented as a loop rather than recursion (code audit 04/07/2026,
    /// point 1): walking up the ancestors is a pure tail
    /// recursion, converted here with no behavior change to avoid any
    /// dependency on tail-call optimization (not guaranteed by Rust).
    #[must_use]
    pub fn is_mainline(&self, id: usize) -> bool {
        let mut current = id;
        loop {
            let Some(node) = self.nodes.get(&current) else {
                return false;
            };

            let siblings: &[usize] = match node.parent {
                Some(pid) => match self.nodes.get(&pid) {
                    Some(p) => &p.children,
                    None => return false, // normally impossible inconsistency
                },
                None => &self.roots,
            };

            if siblings.first() != Some(&current) {
                return false;
            }

            match node.parent {
                Some(pid) => current = pid,
                None => return true,
            }
        }
    }

    /// Flattens the tree into a list of [`FlatNode`]s, in standard PGN
    /// reading order: for a given node, its "main line" next move
    /// (`children[0]`) is displayed immediately after it, then each
    /// variation (`children[1..]`) is displayed in full (recursively,
    /// depth + 1) *before* continuing the main line — exactly
    /// like `2. Nf3 (2. Nc3 Nc6 3. Bc4) Nc6 3. Bb5` in PGN text.
    ///
    /// Root(s): `roots()[0]` (main line of the game) is flattened at
    /// depth 0; any following root (alternative game from the
    /// first move) is treated as a variation at depth 1.
    ///
    /// PHASE 16, Step 3: structure produced to prepare the display of
    /// variations (Step 4) — not yet consumed by the Slint UI.
    #[must_use]
    pub fn flatten(&self) -> Vec<FlatNode> {
        let mut out = Vec::with_capacity(self.nodes.len());

        if let Some((&first_root, other_roots)) = self.roots.split_first() {
            out.push(FlatNode {
                node_id: first_root,
                depth: 0,
                parent_id: None,
                // `roots()[0]` is *always* the main line, by
                // construction (see `roots()` doc) — equivalent to
                // `self.is_mainline(first_root)`, which would have returned
                // `true` immediately here anyway (root with no parent),
                // but avoids the call to stay consistent with the rest of
                // this method (see perf note below).
                is_mainline: true,
            });
            self.flatten_expand(first_root, 0, true, &mut out);

            for &root_variation in other_roots {
                out.push(FlatNode {
                    node_id: root_variation,
                    depth: 1,
                    parent_id: None,
                    is_mainline: false,
                });
                self.flatten_expand(root_variation, 1, false, &mut out);
            }
        }

        out
    }

    /// Extends `out` with the descendants of `node_id` (already present in
    /// `out`, on the main line iff `node_is_mainline`), by first pushing
    /// the "main line" next move, then each variation in
    /// full, before continuing with the main line — see
    /// [`Self::flatten`].
    ///
    /// Implemented with an explicit stack rather than recursion (code audit
    /// 04/07/2026, point 1): unlike a simple tail recursion,
    /// this traversal must *fully* expand each variation before
    /// moving to the next one and then resuming the main line — two
    /// kinds of instructions are therefore stacked ([`FlattenTask::Variation`]
    /// pushes a variation's record then schedules its
    /// expansion, [`FlattenTask::Expand`] expands the children of a
    /// node already present in `out`), stacked in reverse order so that
    /// popping (LIFO) reproduces exactly the order of the old
    /// recursive version — see `test_flatten_*` and
    /// `test_flatten_extremely_long_mainline_does_not_overflow_stack` for
    /// non-regression. Without this conversion, an extremely
    /// long main line (pathological PGN import, engine-vs-engine
    /// tournament chaining thousands of moves) risked a stack
    /// overflow, since `flatten()`/`flatten_expand()` are called on every
    /// rebuild of the move list (after every move played).
    ///
    /// **Perf note**: `node_is_mainline` is *propagated* along the
    /// traversal (inherited as-is by the "main line" child, always `false`
    /// for a variation) rather than recomputed via [`Self::is_mainline`] at
    /// every node — calling `is_mainline` (which walks up all ancestors)
    /// once per main-line node would have cost O(N²) total
    /// for a line of N moves (discovered while adding the
    /// `test_flatten_extremely_long_mainline_does_not_overflow_stack` test,
    /// 100,000 moves: `cargo test` did not "crash" but stayed stuck
    /// for several minutes). Propagation brings the total cost back to O(N).
    fn flatten_expand(&self, node_id: usize, depth: usize, node_is_mainline: bool, out: &mut Vec<FlatNode>) {
        let mut stack = vec![FlattenTask::Expand { node_id, depth, node_is_mainline }];

        while let Some(task) = stack.pop() {
            match task {
                FlattenTask::Variation { node_id, depth, parent_id } => {
                    out.push(FlatNode {
                        node_id,
                        depth,
                        parent_id: Some(parent_id),
                        is_mainline: false,
                    });
                    stack.push(FlattenTask::Expand { node_id, depth, node_is_mainline: false });
                }
                FlattenTask::Expand { node_id, depth, node_is_mainline } => {
                    let Some(node) = self.nodes.get(&node_id) else { continue };
                    let Some((&mainline, variations)) = node.children.split_first() else { continue };

                    out.push(FlatNode {
                        node_id: mainline,
                        depth,
                        parent_id: Some(node_id),
                        // The "main line" child inherits exactly the
                        // status of its parent (see perf note above).
                        is_mainline: node_is_mainline,
                    });

                    // Stacked in reverse order of the desired processing: the
                    // main line must be expanded last (after
                    // ALL variations, each expanded in full),
                    // so its instruction goes to the bottom of the stack; the
                    // variations are stacked backwards so that the first
                    // one (v1) is popped — hence processed — first.
                    stack.push(FlattenTask::Expand { node_id: mainline, depth, node_is_mainline });
                    for &variation in variations.iter().rev() {
                        stack.push(FlattenTask::Variation {
                            node_id: variation,
                            depth: depth + 1,
                            parent_id: node_id,
                        });
                    }
                }
            }
        }
    }

    /// Half-move index of `id` from the start of ITS line (root or
    /// variation root): `0` = first move of the line, `1` = second,
    /// etc. — obtained by walking up the chain of parents.
    ///
    /// For a main-line node, this index matches exactly the index used by
    /// [`crate::history::History::get`] (same
    /// convention: `0` = first move of the game). For a variation
    /// node, it is the index *within that line*, not in the whole
    /// game (a variation starting after move 5 restarts at `0`).
    ///
    /// `None` if `id` is unknown.
    #[must_use]
    pub fn ply_index(&self, id: usize) -> Option<usize> {
        let mut count = 0;
        let mut current = id;
        loop {
            let node = self.nodes.get(&current)?;
            match node.parent {
                Some(pid) => {
                    count += 1;
                    current = pid;
                }
                None => return Some(count),
            }
        }
    }

    /// Pushes into `out` the text of the single move `node_id` (one token: "N.san",
    /// "N...san" or "san"), without dealing with its children — building block of
    /// [`Self::render_continue`]/[`Self::build_variation_blocks`].
    ///
    /// `is_first` triggers the "N..." prefix for a black move that starts
    /// a new reading "segment" (start of a line, or right after an
    /// opening parenthesis) — decision 6. A white move always displays its
    /// number; an isolated black move never does (standard PGN
    /// convention: `2.Nc3 Nc6`, not `2.Nc3 2...Nc6`).
    ///
    /// Takes `node_id`'s ply as a parameter rather than computing it via
    /// [`Self::ply_index`] (an ancestor walk costing `O(depth)`): every
    /// caller ([`Self::render_continue`], [`Self::build_variation_blocks`])
    /// already has it in hand from the traversal, or from computing it once
    /// up front — see [`Self::render_continue`]'s doc for why this matters
    /// (robustness audit 11/07/2026, finding 2.2 follow-up: an earlier
    /// `push_move_text` wrapper that recomputed the ply itself via
    /// `ply_index` on every call was removed, since it turned an `O(N)`
    /// traversal into `O(N²)`).
    fn push_move_text_at_ply(&self, node_id: usize, ply: usize, is_first: bool, out: &mut String) {
        let Some(node) = self.nodes.get(&node_id) else { return };
        let move_number = ply / 2 + 1;
        let is_white = ply.is_multiple_of(2);

        if !out.is_empty() {
            out.push(' ');
        }
        if is_white {
            // write! on a String cannot fail (cf. std impl); the
            // Result is intentionally ignored (clippy::format_push_string).
            let _ = write!(out, "{move_number}.{}", node.record.san);
        } else if is_first {
            let _ = write!(out, "{move_number}...{}", node.record.san);
        } else {
            out.push_str(&node.record.san);
        }

        // NAG (PHASE 16, Step 6.1): glyph glued directly after the SAN,
        // with no space (standard convention, e.g. "e4!" rather than "e4 !").
        if let Some(nag) = node.nag {
            out.push_str(nag.symbol());
        }
    }

    /// Extends `out` with the continuation of the line after `node_id` (whose text has
    /// already been pushed by the caller, see [`Self::push_move_text_at_ply`]) —
    /// textual mirror of [`Self::flatten_expand`]: pushes the next
    /// mainline move first (a single token), then folds each variation into
    /// full inline parentheses, then resumes the main line.
    /// The order is essential: processing variations in full depth
    /// *before* continuing the main line guarantees that a variation
    /// displays right after the move it is an alternative to, rather
    /// than after the rest of the entire game (see doc of
    /// [`Self::flatten_expand`] for the same constraint on the `flatten` side).
    ///
    /// Implemented with an explicit stack rather than recursion (robustness
    /// audit 11/07/2026, finding 2.2 — same fix as
    /// [`Self::flatten_expand`] on 04/07/2026, applied here belatedly:
    /// `render_continue` had been overlooked at the time even though it
    /// walks the exact same tree shape and is called on every rebuild of
    /// the move list, exposing it to the same risk of stack overflow on
    /// an extremely long imported PGN main line). [`RenderTask::Expand`]
    /// mirrors [`FlattenTask::Expand`]; [`RenderTask::OpenVariation`]
    /// mirrors [`FlattenTask::Variation`] (writes the opening `" ("` plus
    /// the variation's first token, then schedules the expansion of its
    /// own continuation, exactly like the original recursive call at the
    /// top of the `for` loop body below used to do); [`RenderTask::CloseParen`]
    /// has no recursive equivalent — it stands in for the `out.push(')')`
    /// statement that used to run *after* the recursive call returned,
    /// which an explicit LIFO stack cannot express directly (the closing
    /// paren must be scheduled to run only once everything nested inside
    /// the variation, however deep, has finished). See
    /// `test_render_continue_extremely_long_mainline_does_not_overflow_stack`
    /// for non-regression, and `test_flatten_extremely_long_mainline_does_not_overflow_stack`
    /// for the equivalent test on `flatten_expand`.
    ///
    /// `node_ply` is `node_id`'s own ply (see [`Self::ply_index`]),
    /// already known by every current caller ([`Self::build_variation_blocks`]
    /// computes it right before calling) — taking it as a parameter
    /// rather than recomputing it here is what lets [`RenderTask`] carry
    /// and propagate ply values down the traversal instead of each task
    /// calling [`Self::ply_index`] itself; see [`RenderTask`]'s doc for
    /// why this matters (the `O(N²)` trap found while testing the stack
    /// fix above).
    fn render_continue(&self, node_id: usize, node_ply: usize, out: &mut String) {
        let mut stack = vec![RenderTask::Expand { node_id, ply: node_ply }];

        while let Some(task) = stack.pop() {
            match task {
                RenderTask::CloseParen => out.push(')'),

                RenderTask::OpenVariation { node_id, ply } => {
                    out.push_str(" (");
                    // Builds the variation's first token in a separate buffer:
                    // `push_move_text_at_ply` inserts a separator space as
                    // soon as `out` is not empty (normal case between two
                    // moves), which would add an unwanted space right after
                    // the "(" just pushed (bug identified and fixed on
                    // 04/07/2026: "( 2...Nf6)" instead of "(2...Nf6)").
                    let mut first_token = String::new();
                    self.push_move_text_at_ply(node_id, ply, true, &mut first_token);
                    out.push_str(&first_token);
                    // Popped (LIFO) in the opposite order they are pushed:
                    // `Expand` must run — and fully complete, including
                    // everything it schedules — before `CloseParen` runs,
                    // so `CloseParen` goes to the bottom.
                    stack.push(RenderTask::CloseParen);
                    stack.push(RenderTask::Expand { node_id, ply });
                }

                RenderTask::Expand { node_id, ply } => {
                    let Some(node) = self.nodes.get(&node_id) else { continue };
                    let Some((&mainline, variations)) = node.children.split_first() else { continue };

                    // Every child of `node_id` — the "main line" one and
                    // every variation alike — shares the same `parent`
                    // (`node_id` itself, see `GameTree`'s doc on the
                    // `children[0]` convention), hence the same ply:
                    // `ply + 1`. Computed once here instead of via
                    // `ply_index` inside `push_move_text`/for each
                    // `OpenVariation` task below — see `RenderTask`'s doc.
                    let child_ply = ply + 1;

                    self.push_move_text_at_ply(mainline, child_ply, false, out);

                    // Stacked in reverse order of the desired processing: the
                    // main line must be resumed last (after ALL variations,
                    // each expanded in full), so its instruction goes to the
                    // bottom of the stack; the variations are stacked
                    // backwards so that the first one is popped — hence
                    // processed — first (same principle as
                    // `flatten_expand`).
                    stack.push(RenderTask::Expand { node_id: mainline, ply: child_ply });
                    for &variation in variations.iter().rev() {
                        stack.push(RenderTask::OpenVariation { node_id: variation, ply: child_ply });
                    }
                }
            }
        }
    }

    /// Builds the list of variation blocks to display (PHASE 16,
    /// Step 4), one block per depth-1 variation — depth-≥2 sub-variations
    /// are already folded into inline parentheses in the text
    /// of the enclosing block (see [`Self::render_continue`]); they don't have
    /// their own block.
    ///
    /// `after_ply` designates the main-line move right after which
    /// the block must be displayed (the move this variation is an
    /// alternative to). Computed via [`Self::ply_index`] of the variation
    /// node itself rather than by [`Self::flatten`]'s traversal order: a
    /// variation and its main-line "sibling" share the same parent,
    /// hence the same `ply_index` by construction — this remains true also
    /// for a root variation (alternative from the first move of the
    /// game, decision 1), which has no parent and where a simple tracking of
    /// the "last mainline ply encountered during the traversal" would give an
    /// incorrect result (`flatten` only visits root-variations after
    /// fully traversing the main line, cf. its documentation).
    ///
    /// A depth-1 node is not necessarily the *start* of a variation
    /// — it can also be the continuation of an already-started variation (the
    /// `children[0]` of a node itself at depth 1, which inherits the
    /// same depth in [`Self::flatten_expand`]). Only the first case
    /// produces a new block; the second is already included in the text of the
    /// enclosing block via [`Self::render_continue`] — without this distinction,
    /// a multi-move variation would produce one extra block per
    /// continuation move (bug identified and fixed on 04/07/2026).
    #[must_use]
    pub fn build_variation_blocks(&self) -> Vec<VariationBlock> {
        let mut blocks = Vec::new();

        for node in self.flatten() {
            if node.depth != 1 || !self.is_variation_start(node.node_id, node.parent_id) {
                continue;
            }
            let Some(after_ply) = self.ply_index(node.node_id) else { continue };

            let mut text = String::new();
            self.push_move_text_at_ply(node.node_id, after_ply, true, &mut text);
            self.render_continue(node.node_id, after_ply, &mut text);
            blocks.push(VariationBlock { after_ply, text, start_node_id: node.node_id });
        }

        blocks
    }

    /// `true` if `id` is one of its parent's `children[1..]` (an
    /// alternative), `false` if it is `children[0]` (a continuation of
    /// its parent's line) — or if `id` is a root (always considered
    /// a starting point, cf. [`Self::build_variation_blocks`]).
    fn is_variation_start(&self, id: usize, parent_id: Option<usize>) -> bool {
        match parent_id {
            Some(pid) => {
                self.nodes.get(&pid).is_some_and(|p| p.children.first() != Some(&id))
            }
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{chess_move::Move, square::Square};

    fn mv(from: &str, to: &str) -> Move {
        Move::normal(
            Square::from_algebraic(from).unwrap(),
            Square::from_algebraic(to).unwrap(),
        )
    }

    fn rec(from: &str, to: &str, san: &str) -> MoveRecord {
        MoveRecord {
            mv: mv(from, to),
            san: san.into(),
            fen_before: String::new(),
            from_book: false,
        }
    }

    // ── Empty tree ────────────────────────────────────────────────────────

    #[test]
    fn test_new_tree_is_empty() {
        let t = GameTree::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.roots().is_empty());
    }

    // ── add_move: roots ───────────────────────────────────────────────

    #[test]
    fn test_add_move_as_root_becomes_mainline() {
        let mut t = GameTree::new();
        let id = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        assert_eq!(t.len(), 1);
        assert_eq!(t.roots().to_vec(), vec![id]);
        assert_eq!(t.node(id).unwrap().record.san, "e4");
        assert!(t.is_mainline(id));
    }

    #[test]
    fn test_second_root_is_variation_not_replacement() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        // No truncation: both coexist, in insertion order.
        assert_eq!(t.len(), 2);
        assert_eq!(t.roots().to_vec(), vec![e4, d4]);
        assert!(t.is_mainline(e4));
        assert!(!t.is_mainline(d4));
    }

    // ── add_move: children ───────────────────────────────────────────────

    #[test]
    fn test_add_move_as_child_appears_in_parent_children() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();

        assert_eq!(t.node(e4).unwrap().children, [e5]);
        assert_eq!(t.node(e5).unwrap().parent, Some(e4));
        assert!(t.is_mainline(e5));
    }

    #[test]
    fn test_add_move_with_unknown_parent_returns_none() {
        let mut t = GameTree::new();
        assert_eq!(t.add_move(Some(999), rec("e2", "e4", "e4")), None);
        assert!(t.is_empty(), "aucune modification en cas de parent inconnu");
    }

    #[test]
    fn test_deep_variation_is_not_mainline_unless_all_ancestors_are() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        // Variation: 2...Nc6 instead of 2...Nf3 (same parent as nf3).
        let nc6 = t.add_move(Some(e5), rec("b8", "c6", "Nc6")).unwrap();
        // Sub-variation of a line mainline up to that point.
        let sub = t.add_move(Some(nf3), rec("b8", "c6", "Nc6")).unwrap();

        assert!(t.is_mainline(e4) && t.is_mainline(e5) && t.is_mainline(nf3));
        assert!(t.is_mainline(sub), "premier enfant d'un nœud mainline reste mainline");
        assert!(!t.is_mainline(nc6), "second enfant = variante, jamais mainline");
    }

    #[test]
    fn test_is_mainline_unknown_id_returns_false() {
        let t = GameTree::new();
        assert!(!t.is_mainline(42));
    }

    // ── promote_to_mainline ──────────────────────────────────────────────

    #[test]
    fn test_promote_root_variation_to_mainline() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        assert!(t.promote_to_mainline(d4));

        assert_eq!(t.roots().to_vec(), vec![d4, e4]);
        assert!(t.is_mainline(d4));
        assert!(!t.is_mainline(e4));
    }

    #[test]
    fn test_promote_nested_variation_to_mainline() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let nf3 = t.add_move(Some(e4), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e4), rec("b1", "c3", "Nc3")).unwrap();

        assert!(t.promote_to_mainline(nc3));

        assert_eq!(t.node(e4).unwrap().children, [nc3, nf3]);
        assert!(t.is_mainline(nc3));
        assert!(!t.is_mainline(nf3));
    }

    #[test]
    fn test_promote_already_mainline_is_noop_but_returns_true() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        assert!(t.promote_to_mainline(e4));
        assert_eq!(t.roots().to_vec(), vec![e4]);
    }

    #[test]
    fn test_promote_unknown_id_returns_false() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        assert!(!t.promote_to_mainline(999));
        assert_eq!(t.roots().to_vec(), vec![e4], "aucune modification en cas d'id inconnu");
    }

    // ── remove_subtree ───────────────────────────────────────────────────

    #[test]
    fn test_remove_leaf_node() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();

        assert!(t.remove_subtree(e5));

        assert!(t.node(e5).is_none());
        assert!(t.node(e4).unwrap().children.is_empty());
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_remove_subtree_removes_all_descendants() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc6 = t.add_move(Some(nf3), rec("b8", "c6", "Nc6")).unwrap();

        assert!(t.remove_subtree(e5));

        assert!(t.node(e5).is_none());
        assert!(t.node(nf3).is_none());
        assert!(t.node(nc6).is_none());
        assert_eq!(t.len(), 1); // only e4 remains
        assert!(t.node(e4).unwrap().children.is_empty());
    }

    #[test]
    fn test_remove_root_variation_leaves_other_roots_intact() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        assert!(t.remove_subtree(d4));

        assert_eq!(t.roots().to_vec(), vec![e4]);
        assert!(t.node(d4).is_none());
    }

    #[test]
    fn test_remove_mainline_promotes_next_sibling_automatically() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        assert!(t.remove_subtree(e4));

        // d4 was a variation, it becomes the only remaining root hence
        // mechanically the main line.
        assert_eq!(t.roots().to_vec(), vec![d4]);
        assert!(t.is_mainline(d4));
    }

    #[test]
    fn test_remove_unknown_id_returns_false() {
        let mut t = GameTree::new();
        let _e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        assert!(!t.remove_subtree(999));
        assert_eq!(t.len(), 1);
    }

    // ── node_mut: comment and NAG ────────────────────────────────────

    #[test]
    fn test_node_mut_sets_comment_and_nag() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        {
            let node = t.node_mut(e4).unwrap();
            node.comment = Some("Meilleur premier coup selon la théorie".into());
            node.nag = Some(Nag::Good);
        }

        let node = t.node(e4).unwrap();
        assert_eq!(node.comment.as_deref(), Some("Meilleur premier coup selon la théorie"));
        assert_eq!(node.nag, Some(Nag::Good));
    }

    #[test]
    fn test_node_mut_unknown_id_returns_none() {
        let mut t = GameTree::new();
        assert!(t.node_mut(999).is_none());
    }

    #[test]
    fn test_comment_and_nag_default_to_none() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        let node = t.node(e4).unwrap();
        assert_eq!(node.comment, None);
        assert_eq!(node.nag, None);
    }

    // ── Nag ──────────────────────────────────────────────────────────────

    #[test]
    fn test_nag_symbols() {
        assert_eq!(Nag::Brilliant.symbol(), "!!");
        assert_eq!(Nag::Good.symbol(), "!");
        assert_eq!(Nag::Interesting.symbol(), "!?");
        assert_eq!(Nag::Dubious.symbol(), "?!");
        assert_eq!(Nag::Mistake.symbol(), "?");
        assert_eq!(Nag::Blunder.symbol(), "??");
    }

    #[test]
    fn test_nag_code_roundtrip() {
        for nag in [
            Nag::Brilliant,
            Nag::Good,
            Nag::Interesting,
            Nag::Dubious,
            Nag::Mistake,
            Nag::Blunder,
        ] {
            assert_eq!(Nag::from_code(nag.code()), Some(nag));
        }
    }

    #[test]
    fn test_nag_standard_pgn_codes() {
        // Numeric codes fixed by the PGN standard ($1 to $6) — checked
        // explicitly to avoid any silent regression at
        // Step 7 (PGN export/import).
        assert_eq!(Nag::Good.code(), 1);
        assert_eq!(Nag::Mistake.code(), 2);
        assert_eq!(Nag::Brilliant.code(), 3);
        assert_eq!(Nag::Blunder.code(), 4);
        assert_eq!(Nag::Interesting.code(), 5);
        assert_eq!(Nag::Dubious.code(), 6);
    }

    #[test]
    fn test_nag_from_code_out_of_range_is_none() {
        assert_eq!(Nag::from_code(0), None);
        assert_eq!(Nag::from_code(7), None);
        assert_eq!(Nag::from_code(255), None);
    }

    // ── flatten (PHASE 16, Step 3) ──────────────────────────────────────────

    /// Reconstructs the sequence of SANs in the order produced by `flatten()`,
    /// with the associated depth — easier to assert than a list of
    /// raw `FlatNode`s (the `node_id`s are not predictable in advance).
    fn flat_sans(t: &GameTree) -> Vec<(String, usize)> {
        t.flatten()
            .into_iter()
            .map(|f| (t.node(f.node_id).unwrap().record.san.clone(), f.depth))
            .collect()
    }

    #[test]
    fn test_flatten_empty_tree() {
        let t = GameTree::new();
        assert!(t.flatten().is_empty());
    }

    #[test]
    fn test_flatten_linear_mainline_only() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();

        assert_eq!(
            flat_sans(&t),
            vec![("e4".to_string(), 0), ("e5".to_string(), 0), ("Nf3".to_string(), 0)]
        );
    }

    #[test]
    fn test_flatten_matches_is_mainline_and_parent() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();

        let flat = t.flatten();
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].node_id, e4);
        assert_eq!(flat[0].parent_id, None);
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].is_mainline);
        assert_eq!(flat[1].node_id, e5);
        assert_eq!(flat[1].parent_id, Some(e4));
        assert_eq!(flat[1].depth, 0);
        assert!(flat[1].is_mainline);
    }

    #[test]
    fn test_flatten_single_variation_appears_after_mainline_move_before_continuation() {
        // 1.e4 e5 2.Nf3 (2.Nc3) Nc6: the variation follows "Nf3" and precedes "Nc6"
        // (PGN standard — cf. `GameTree::flatten` doc).
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap(); // variation
        t.add_move(Some(nf3), rec("b8", "c6", "Nc6")).unwrap();

        assert_eq!(
            flat_sans(&t),
            vec![
                ("e4".to_string(), 0),
                ("e5".to_string(), 0),
                ("Nf3".to_string(), 0),
                ("Nc3".to_string(), 1),
                ("Nc6".to_string(), 0),
            ]
        );
    }

    #[test]
    fn test_flatten_variation_with_its_own_continuation_stays_fully_nested() {
        // 1.e4 e5 2.Nf3 (2.Nc3 Nc6 3.Bc4) Nc6 3.Bb5: the continuation of the
        // variation (Nc6 3.Bc4) stays at the same depth as Nc3, before
        // returning to the main line.
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        let nc6_var = t.add_move(Some(nc3), rec("b8", "c6", "Nc6")).unwrap();
        t.add_move(Some(nc6_var), rec("f1", "c4", "Bc4")).unwrap();
        let nc6_main = t.add_move(Some(nf3), rec("b8", "c6", "Nc6")).unwrap();
        t.add_move(Some(nc6_main), rec("f1", "b5", "Bb5")).unwrap();

        assert_eq!(
            flat_sans(&t),
            vec![
                ("e4".to_string(), 0),
                ("e5".to_string(), 0),
                ("Nf3".to_string(), 0),
                ("Nc3".to_string(), 1),
                ("Nc6".to_string(), 1),
                ("Bc4".to_string(), 1),
                ("Nc6".to_string(), 0),
                ("Bb5".to_string(), 0),
            ]
        );
    }

    #[test]
    fn test_flatten_nested_variation_of_variation_is_depth_two() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let _nf3 = t.add_move(Some(e4), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e4), rec("b1", "c3", "Nc3")).unwrap(); // depth 1
        t.add_move(Some(nc3), rec("d2", "d4", "d4")).unwrap(); // mainline of the variation, depth 1
        t.add_move(Some(nc3), rec("g2", "g3", "g3")).unwrap(); // variation of the variation, depth 2

        assert_eq!(
            flat_sans(&t),
            vec![
                ("e4".to_string(), 0),
                ("Nf3".to_string(), 0),
                ("Nc3".to_string(), 1),
                ("d4".to_string(), 1),
                ("g3".to_string(), 2),
            ]
        );
    }

    #[test]
    fn test_flatten_extremely_long_mainline_does_not_overflow_stack() {
        // Code audit 04/07/2026, point 1: `flatten()`/`flatten_expand()`
        // (and `is_mainline()`) were recursive — an extremely
        // long main line (pathological PGN import, or an engine-vs-engine
        // tournament session chaining thousands of moves
        // without a reset) risked a stack overflow, since these functions
        // are called on every rebuild of the move list. This test
        // builds a line far beyond any real game (no
        // tournament game comes close to this number of moves) and verifies that the
        // iterative traversal completes without panicking.
        const DEPTH: usize = 100_000;
        let mut t = GameTree::new();
        let mut parent: Option<usize> = None;
        for i in 0..DEPTH {
            let (from, to) = if i.is_multiple_of(2) { ("a1", "a2") } else { ("a2", "a1") };
            parent = Some(t.add_move(parent, rec(from, to, "x")).unwrap());
        }

        let flat = t.flatten();
        assert_eq!(flat.len(), DEPTH);
        assert!(flat.iter().all(|f| f.depth == 0 && f.is_mainline));
    }

    #[test]
    fn test_render_continue_extremely_long_mainline_does_not_overflow_stack() {
        // Robustness audit 11/07/2026, finding 2.2: `render_continue` was
        // still recursive — the exact same risk `flatten_expand` was fixed
        // for on 04/07/2026 (see the test just above), but it had been
        // overlooked at the time even though it walks the same tree shape
        // and is called on every rebuild of the variation blocks (after
        // every move played). A root *variation* is used here (rather
        // than the main line itself) because `build_variation_blocks` —
        // the only public entry point that reaches `render_continue` — only
        // calls it on depth-1 variation-start nodes; the variation's own
        // continuation is then chained far beyond any real game, which
        // exercises exactly the part of `render_continue` that used to be
        // a recursive tail call (`self.render_continue(mainline, out)`),
        // repeated `DEPTH` times deep in the old implementation.
        //
        // Follow-up (same day): making the traversal iterative fixed the
        // crash, but this test then *hung* instead of finishing, because
        // `push_move_text` was calling `self.ply_index(node_id)` — an
        // O(depth) walk up the `parent` chain — for every one of the
        // `DEPTH` nodes visited, i.e. O(DEPTH²) total work (~10 billion
        // steps for DEPTH = 100_000). This cost already existed in the old
        // recursive code too, but was masked: the stack overflow always
        // struck first, long before the traversal got deep enough for the
        // O(n²) term to dominate. The fix mirrors the `node_is_mainline`
        // propagation already used in `flatten_expand`: `ply` is now
        // threaded through `RenderTask` and computed once per node as
        // `parent_ply + 1`, restoring O(DEPTH) total cost.
        const DEPTH: usize = 100_000;
        let mut t = GameTree::new();
        let _e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap(); // main line, untouched
        let variation_root = t.add_move(None, rec("d2", "d4", "d4")).unwrap(); // root variation
        let mut parent = Some(variation_root);
        for i in 0..DEPTH {
            let (from, to) = if i.is_multiple_of(2) { ("a1", "a2") } else { ("a2", "a1") };
            parent = Some(t.add_move(parent, rec(from, to, "x")).unwrap());
        }

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_node_id, variation_root);
        assert!(blocks[0].text.starts_with("1.d4"));
        // No parenthesis: the chained continuation is a plain mainline
        // (no nested variation of its own), so the folded text must be one
        // long flat token sequence, not truncated partway through.
        assert!(!blocks[0].text.contains('('));
    }

    #[test]
    fn test_flatten_multiple_root_variations() {
        // Two possible first moves from the start of the game: e4 (main
        // line) and d4 (variation from the first move, PHASE 16 decision 1).
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        let flat = t.flatten();
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].node_id, e4);
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].is_mainline);
        assert_eq!(flat[1].node_id, d4);
        assert_eq!(flat[1].depth, 1);
        assert_eq!(flat[1].parent_id, None);
        assert!(!flat[1].is_mainline);
    }

    #[test]
    fn test_flatten_visits_every_node_exactly_once() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        t.add_move(None, rec("d2", "d4", "d4")).unwrap();

        let flat = t.flatten();
        assert_eq!(flat.len(), t.len());
        let mut ids: Vec<usize> = flat.iter().map(|f| f.node_id).collect();
        ids.sort_unstable();
        let mut expected: Vec<usize> = (0..t.len()).collect();
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    // ── ply_index (PHASE 16, Step 4) ────────────────────────────────────────

    #[test]
    fn test_ply_index_along_mainline_matches_history_convention() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();

        assert_eq!(t.ply_index(e4), Some(0));
        assert_eq!(t.ply_index(e5), Some(1));
        assert_eq!(t.ply_index(nf3), Some(2));
    }

    #[test]
    fn test_ply_index_variation_restarts_from_its_own_root() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let d4 = t.add_move(None, rec("d2", "d4", "d4")).unwrap(); // root variation

        assert_eq!(t.ply_index(e4), Some(0));
        assert_eq!(t.ply_index(d4), Some(0), "variante dès le 1er coup : recommence à 0");
    }

    #[test]
    fn test_ply_index_unknown_id_is_none() {
        let t = GameTree::new();
        assert_eq!(t.ply_index(42), None);
    }

    // ── build_variation_blocks (PHASE 16, Step 4) ──────────────────────────

    #[test]
    fn test_build_variation_blocks_empty_tree() {
        let t = GameTree::new();
        assert!(t.build_variation_blocks().is_empty());
    }

    #[test]
    fn test_build_variation_blocks_no_variations() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();

        assert!(t.build_variation_blocks().is_empty());
    }

    #[test]
    fn test_build_variation_blocks_single_white_alternative() {
        // 1.e4 e5 2.Nf3 (2.Nc3)
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].after_ply, t.ply_index(nf3).unwrap());
        assert_eq!(blocks[0].text, "2.Nc3");
    }

    // ── NAG in the rendered text (PHASE 16, Step 6.1) ────────────────────────

    #[test]
    fn test_push_move_text_appends_nag_symbol_directly_after_san() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        t.node_mut(e4).unwrap().nag = Some(Nag::Blunder);

        let mut out = String::new();
        t.push_move_text_at_ply(e4, 0, true, &mut out);
        assert_eq!(out, "1.e4??", "le glyphe doit être collé, sans espace");
    }

    #[test]
    fn test_push_move_text_no_nag_unaffected() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();

        let mut out = String::new();
        t.push_move_text_at_ply(e4, 0, true, &mut out);
        assert_eq!(out, "1.e4");
    }

    #[test]
    fn test_build_variation_blocks_includes_nag_of_variation_start() {
        // 1.e4 e5 2.Nf3 (2.Nc3!)
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        t.node_mut(nc3).unwrap().nag = Some(Nag::Good);

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "2.Nc3!");
    }

    #[test]
    fn test_build_variation_blocks_exposes_start_node_id() {
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_node_id, nc3);
    }

    #[test]
    fn test_build_variation_blocks_isolated_black_move_gets_ellipsis() {
        // 1.e4 e5 (1...c5)
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e4), rec("c7", "c5", "c5")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].after_ply, t.ply_index(e5).unwrap());
        assert_eq!(blocks[0].text, "1...c5");
    }

    #[test]
    fn test_build_variation_blocks_continuation_has_no_number_for_black() {
        // 1.e4 e5 2.Nf3 (2.Nc3 Nc6)
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        t.add_move(Some(nc3), rec("b8", "c6", "Nc6")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "2.Nc3 Nc6");
    }

    #[test]
    fn test_build_variation_blocks_nested_subvariation_folded_inline() {
        // 1.e4 e5 2.Nf3 (2.Nc3 Nc6 (2...Nf6)) Nc6 3.Bb5
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        let nc3 = t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        let nc6_var = t.add_move(Some(nc3), rec("b8", "c6", "Nc6")).unwrap();
        t.add_move(Some(nc3), rec("g8", "f6", "Nf6")).unwrap(); // sub-variation
        let nc6_main = t.add_move(Some(nf3), rec("b8", "c6", "Nc6")).unwrap();
        t.add_move(Some(nc6_main), rec("f1", "b5", "Bb5")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 1, "la sous-variante n'a pas son propre bloc");
        assert_eq!(blocks[0].after_ply, t.ply_index(nf3).unwrap());
        assert_eq!(blocks[0].text, "2.Nc3 Nc6 (2...Nf6)");
        let _ = nc6_var; // used only to build the tree
    }

    #[test]
    fn test_build_variation_blocks_multiple_variations_at_different_plies() {
        // 1.e4 (1.d4) e5 (1...c5): two independent variations, each
        // attached to the ply of its own "main line" sibling via
        // `ply_index` — the `flatten()` order differs for a root
        // variation (visited after the entire main line), so this test
        // looks up each block by content rather than by position.
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        t.add_move(None, rec("d2", "d4", "d4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        t.add_move(Some(e4), rec("c7", "c5", "c5")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 2);

        let d4_block = blocks.iter().find(|b| b.text == "1.d4").unwrap();
        assert_eq!(d4_block.after_ply, t.ply_index(e4).unwrap());

        let c5_block = blocks.iter().find(|b| b.text == "1...c5").unwrap();
        assert_eq!(c5_block.after_ply, t.ply_index(e5).unwrap());
    }

    #[test]
    fn test_build_variation_blocks_multiple_siblings_same_ply_produce_two_blocks() {
        // 1.e4 e5 2.Nf3 (2.Nc3) (2.Bc4): two distinct variations at the same point.
        let mut t = GameTree::new();
        let e4 = t.add_move(None, rec("e2", "e4", "e4")).unwrap();
        let e5 = t.add_move(Some(e4), rec("e7", "e5", "e5")).unwrap();
        let nf3 = t.add_move(Some(e5), rec("g1", "f3", "Nf3")).unwrap();
        t.add_move(Some(e5), rec("b1", "c3", "Nc3")).unwrap();
        t.add_move(Some(e5), rec("f1", "c4", "Bc4")).unwrap();

        let blocks = t.build_variation_blocks();
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().all(|b| b.after_ply == t.ply_index(nf3).unwrap()));
        assert_eq!(blocks[0].text, "2.Nc3");
        assert_eq!(blocks[1].text, "2.Bc4");
    }
}
