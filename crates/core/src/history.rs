//! History of moves in a game.
//!
//! [`History`] stores the sequence of [`MoveRecord`]s played since the
//! initial position. Each record contains the move, its SAN notation, and the
//! FEN of the position *before* the move (for navigation/undo).
//!
//! **PHASE 16, Step 2**: `History` is now internally backed by
//! [`crate::game_tree::GameTree`] rather than a plain `Vec<MoveRecord>` —
//! preparation for the arrival of variations (Steps 3+). This module's public
//! API remains strictly unchanged in its usage (same methods, same
//! behavior for all current callers) with one nuance:
//! [`History::records`] now returns a cloned `Vec<MoveRecord>` rather
//! than a `&[MoveRecord]` slice (a `HashMap`-backed tree cannot expose
//! a contiguous slice) — all existing uses (`.len()`, `.iter()`,
//! `.last()`) remain valid without modification thanks to `Deref<Target =
//! [MoveRecord]>` on `Vec`. Since Step 5, [`History::branch_at`] allows
//! creating a variation (new branch) without ever removing nodes from
//! the tree — `path` can therefore now point to any line
//! of the tree, not just the very first one ever recorded.

use crate::game_tree::GameTree;
use crate::types::chess_move::Move;

// ---------------------------------------------------------------------------
// MoveRecord
// ---------------------------------------------------------------------------

/// A move recorded in the history.
#[derive(Debug, Clone)]
pub struct MoveRecord {
    /// The move played.
    pub mv:         Move,
    /// SAN notation of the move.
    pub san:        String,
    /// FEN of the position **before** this move.
    pub fen_before: String,
    /// `true` if this move was played automatically from a Polyglot
    /// opening book rather than computed by the engine or played by a
    /// human (see `crates/core/src/polyglot.rs`, PHASE 15). `false` by
    /// default — updated by the caller (GUI), which alone knows whether the move
    /// comes from the book.
    pub from_book:  bool,
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// Ordered sequence of moves played since the initial position.
///
/// Internally backed by a [`GameTree`] (PHASE 16, Step 2) rather than a
/// `Vec<MoveRecord>`: `path` holds the node identifier of each move
/// played, in the current linear order. `push`/`pop` only ever act at the
/// tip of `path`; [`Self::branch_at`] (Step 5) is the only operation
/// that can make `path` point to a different line (a variation)
/// without ever removing nodes from `tree`.
#[derive(Debug, Clone, Default)]
pub struct History {
    tree: GameTree,
    path: Vec<usize>,
}

impl History {
    /// Creates an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a record to the end of the history.
    ///
    /// # Panics
    /// Does not panic in practice: the last element of `path` (if it
    /// exists) was always inserted into `tree` by a previous call to
    /// this same method, it cannot have disappeared in the meantime.
    pub fn push(&mut self, record: MoveRecord) {
        let parent = self.path.last().copied();
        let id = self
            .tree
            .add_move(parent, record)
            .expect("le parent (dernier élément de `path`) existe toujours dans `tree`");
        self.path.push(id);
    }

    /// Removes and returns the last record, or `None` if empty.
    ///
    /// # Panics
    /// Does not panic in practice: an `id` present in `path` is always
    /// present in `tree` (invariant maintained by all methods of
    /// this struct).
    pub fn pop(&mut self) -> Option<MoveRecord> {
        let id = self.path.pop()?;
        let record = self
            .tree
            .node(id)
            .expect("id présent dans `path` donc présent dans `tree`")
            .record
            .clone();
        self.tree.remove_subtree(id);
        Some(record)
    }

    /// Number of recorded moves.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.path.len()
    }

    /// Returns `true` if no move has been played.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    /// All records, in chronological order.
    ///
    /// Returns a cloned `Vec` (not a `&[MoveRecord]` slice): a
    /// `HashMap`-backed tree cannot expose a contiguous region in memory.
    /// All current uses (`.len()`, `.iter()`, `.last()`) remain
    /// valid without modification thanks to `Deref<Target = [MoveRecord]>` on
    /// `Vec`. For a position-by-position traversal without cloning (a
    /// performance-sensitive path, e.g. the navigation slider), prefer
    /// [`Self::moves`].
    ///
    /// # Panics
    /// Does not panic in practice: every `id` of `path` is always
    /// valid in `tree` (same invariant as [`Self::push`]/[`Self::pop`]).
    #[must_use]
    pub fn records(&self) -> Vec<MoveRecord> {
        self.path
            .iter()
            .map(|&id| self.tree.node(id).expect("id de `path` toujours valide").record.clone())
            .collect()
    }

    /// Last record, or `None` if empty.
    #[must_use]
    pub fn last(&self) -> Option<&MoveRecord> {
        let id = *self.path.last()?;
        self.tree.node(id).map(|n| &n.record)
    }

    /// Last record (mutable access), or `None` if empty.
    ///
    /// Deliberately restricted scope: only allows modifying the
    /// metadata of an already-played record (e.g. `from_book`), never
    /// adding/removing one — see [`Self::push`]/[`Self::pop`] for that.
    pub fn last_mut(&mut self) -> Option<&mut MoveRecord> {
        let id = *self.path.last()?;
        self.tree.node_mut(id).map(|n| &mut n.record)
    }

    /// Record at the given index, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&MoveRecord> {
        let id = *self.path.get(index)?;
        self.tree.node(id).map(|n| &n.record)
    }

    /// Sequence of all moves played, without cloning (walks the tree
    /// directly along `path`) — to be preferred over
    /// [`Self::records`]`().iter().map(|r| r.mv)` on performance-sensitive
    /// paths (e.g. `GameState::position_at` during a slider
    /// drag, cf. perf audit 02/07/2026).
    ///
    /// # Panics
    /// Does not panic in practice: see [`Self::records`].
    pub fn moves(&self) -> impl Iterator<Item = Move> + '_ {
        self.path.iter().map(move |&id| {
            self.tree.node(id).expect("id de `path` toujours valide").record.mv
        })
    }

    /// Sequence of SAN notations, without cloning.
    ///
    /// # Panics
    /// Does not panic in practice: see [`Self::records`].
    pub fn san_list(&self) -> impl Iterator<Item = &str> {
        self.path.iter().map(move |&id| {
            self.tree.node(id).expect("id de `path` toujours valide").record.san.as_str()
        })
    }

    // -----------------------------------------------------------------------
    // PHASE 16, Step 3: access to the underlying tree
    // -----------------------------------------------------------------------
    //
    // Additive API (does not affect any existing caller) preparing
    // variation support: gives `GameController` a stable node identifier
    // (rather than a plain ply index) and read access to the tree
    // to flatten it into lines with depth/parentage (see
    // `crate::game_tree::GameTree::flatten`).

    /// Read access to the underlying move tree.
    #[must_use]
    pub fn tree(&self) -> &GameTree {
        &self.tree
    }

    /// Write access to the underlying move tree (PHASE 16, Step 6.1).
    ///
    /// Gives access to `node_mut`/`promote_to_mainline`/`remove_subtree` for
    /// context-menu actions (NAG, comment, promotion,
    /// deletion) — these operations never modify `path` directly
    /// (unlike `push`/`pop`/`branch_at`), the caller remains
    /// responsible for `path`'s consistency if it removes a node found
    /// there (see `GameController`, planned for Step 6.2 for deletion).
    pub fn tree_mut(&mut self) -> &mut GameTree {
        &mut self.tree
    }

    /// Node identifier of the move at the given index (same indexing as
    /// [`Self::get`]), or `None` if out of bounds.
    #[must_use]
    pub fn node_id_at(&self, index: usize) -> Option<usize> {
        self.path.get(index).copied()
    }

    /// Node identifier of the last move played (tip of the current
    /// line), or `None` if the history is empty.
    #[must_use]
    pub fn last_node_id(&self) -> Option<usize> {
        self.path.last().copied()
    }

    // -----------------------------------------------------------------------
    // PHASE 16, Step 5: variation creation
    // -----------------------------------------------------------------------

    /// Adds `record` as a new branch from the move at index `ply` of
    /// the active line, and makes this branch the new active line.
    ///
    /// Unlike [`Self::push`] (which only acts at the tip of `path`),
    /// `branch_at` can be called while `path` contains moves
    /// beyond `ply`: these moves are **never removed from the tree**
    /// (PHASE 16 decision #1 — no silent truncation). `path` is
    /// truncated to `ply + 1` elements, then the new node is appended to it, which
    /// makes the freshly created variation the active line now
    /// exposed by `len`/`get`/`moves`/etc.
    ///
    /// The new branch is also promoted to the `children[0]` position of
    /// the node at index `ply` (see [`crate::game_tree::GameTree::promote_to_mainline`]):
    /// since it becomes the line actively played/displayed in the
    /// main columns of the history, it must also be what
    /// [`crate::game_tree::GameTree::flatten`]/`build_variation_blocks`
    /// treat as the structural main line — without this
    /// promotion, the old continuation (still `children[0]`) would wrongly remain
    /// invisible (never flagged as a variation) while the
    /// actively played branch would be wrongly displayed twice as a variation
    /// of itself. The old continuation, meanwhile, becomes a genuinely
    /// real variation (visible, collapsible) rather than the "promote to main
    /// line" sense of decision 2 (which remains a manual action,
    /// via right-click, reserved for Step 6, to re-choose *after the fact*
    /// which of several already-existing variations serves as reference).
    ///
    /// Returns the identifier of the new node, or `None` if `ply` is out
    /// of bounds (no modification made in that case).
    ///
    /// # Panics
    /// Does not panic in practice: `parent` comes from `self.path.get(ply)`,
    /// so it is always present in `tree` (same invariant as
    /// [`Self::push`]).
    pub fn branch_at(&mut self, ply: usize, record: MoveRecord) -> Option<usize> {
        let &parent = self.path.get(ply)?;
        let id = self
            .tree
            .add_move(Some(parent), record)
            .expect("parent trouvé dans `path` donc toujours présent dans `tree`");
        self.tree.promote_to_mainline(id);
        self.path.truncate(ply + 1);
        self.path.push(id);
        Some(id)
    }

    // -----------------------------------------------------------------------
    // PHASE 16, Step 6.2: promoting and deleting a variation
    // -----------------------------------------------------------------------

    /// Promotes the variation starting at `node_id` to the main line
    /// (PHASE 16 decisions #2/7 — right-click "Promote to main
    /// line").
    ///
    /// Reorders the tree via [`crate::game_tree::GameTree::promote_to_mainline`]
    /// (a single level, see its documentation) **and** realigns `path` on the
    /// new structural line: the ancestors of `node_id` remain
    /// unchanged (this node is only promoted from its parent, never the whole
    /// lineage up to the root — same limitation as `GameTree::promote_to_mainline`),
    /// then `node_id` itself is added, then its own already-recorded
    /// continuation (`children[0]` repeated to the end). This continuation
    /// is necessarily consistent: every node of the tree was on `path`
    /// at the time it was created (only [`Self::branch_at`] can demote an
    /// already-recorded continuation to a variation, without ever modifying it).
    ///
    /// Without this `path` realignment, the old active line would keep
    /// appearing in the history's main columns (GUI side,
    /// `GameController::build_move_rows`) while now being
    /// structurally relegated to a variation, causing a duplicate display
    /// — the same pitfall fixed in [`Self::branch_at`] at Step 5.
    ///
    /// Returns `false` if `node_id` is unknown (no modification).
    pub fn promote_to_mainline(&mut self, node_id: usize) -> bool {
        if !self.tree.promote_to_mainline(node_id) {
            return false;
        }

        let mut ancestors = Vec::new();
        let mut current = self.tree.node(node_id).and_then(|n| n.parent);
        while let Some(pid) = current {
            ancestors.push(pid);
            current = self.tree.node(pid).and_then(|n| n.parent);
        }
        ancestors.reverse();

        let mut new_path = ancestors;
        let mut cur = node_id;
        loop {
            new_path.push(cur);
            let Some(next) = self.tree.node(cur).and_then(|n| n.children.first().copied()) else {
                break;
            };
            cur = next;
        }

        self.path = new_path;
        true
    }

    /// Removes the variation starting at `node_id` (and all its
    /// descendants) — PHASE 16 decision #7 ("Remove this variation").
    ///
    /// Silently refuses (`false`, no modification) if `node_id`
    /// belongs to `path` (the actively played line): this context menu
    /// never in practice targets a move of the active line (restriction
    /// already enforced on the GUI side, see decision 7); this guard
    /// prevents an incorrect call from corrupting `path` by removing a node
    /// it still references.
    pub fn remove_variation(&mut self, node_id: usize) -> bool {
        if self.path.contains(&node_id) {
            return false;
        }
        self.tree.remove_subtree(node_id)
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

    #[test]
    fn test_history_empty() {
        let h = History::new();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert!(h.last().is_none());
    }

    #[test]
    fn test_history_push_and_len() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: "start".into(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: "after_e4".into(), from_book: false });
        assert_eq!(h.len(), 2);
        assert!(!h.is_empty());
    }

    #[test]
    fn test_history_pop() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: "start".into(), from_book: false });
        let rec = h.pop().unwrap();
        assert_eq!(rec.san, "e4");
        assert!(h.is_empty());
    }

    #[test]
    fn test_history_san_list() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "e5"]);
    }

    #[test]
    fn test_history_get() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });
        assert_eq!(h.get(0).unwrap().san, "Nf3");
        assert!(h.get(1).is_none());
    }

    #[test]
    fn test_move_record_from_book_flag() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: true });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        assert!(h.get(0).unwrap().from_book);
        assert!(!h.get(1).unwrap().from_book);
    }

    #[test]
    fn test_last_mut_flips_from_book() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        h.last_mut().unwrap().from_book = true;

        // Only the last record is affected.
        assert!(!h.get(0).unwrap().from_book);
        assert!(h.get(1).unwrap().from_book);
    }

    #[test]
    fn test_last_mut_none_when_empty() {
        let mut h = History::new();
        assert!(h.last_mut().is_none());
    }

    // ── PHASE 16, Step 2: migration to GameTree ─────────────────────────
    // Tests added specifically to validate the non-regression of the
    // internal implementation switch (Vec<MoveRecord> → GameTree).

    #[test]
    fn test_records_returns_all_moves_in_order() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });

        let records = h.records();
        let sans: Vec<&str> = records.iter().map(|r| r.san.as_str()).collect();
        assert_eq!(sans, ["e4", "e5", "Nf3"]);
        assert_eq!(records.len(), 3);
    }

    #[test]
    fn test_moves_matches_records_mv() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        let via_moves: Vec<Move> = h.moves().collect();
        let via_records: Vec<Move> = h.records().iter().map(|r| r.mv).collect();
        assert_eq!(via_moves, via_records);
    }

    #[test]
    fn test_push_after_pop_reattaches_to_correct_parent() {
        // Simulates undo() followed by a new move: the internal tree must
        // remain consistent (no ghost node, the new move takes the place of
        // the previous one).
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        let popped = h.pop().unwrap();
        assert_eq!(popped.san, "e5");
        assert_eq!(h.len(), 1);

        h.push(MoveRecord { mv: mv("d7", "d5"), san: "d5".into(), fen_before: String::new(), from_book: false });

        assert_eq!(h.len(), 2);
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "d5"]);
        assert_eq!(h.get(0).unwrap().san, "e4");
        assert_eq!(h.get(1).unwrap().san, "d5");
    }

    #[test]
    fn test_pop_all_then_push_starts_clean() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.pop();
        assert!(h.is_empty());

        h.push(MoveRecord { mv: mv("d2", "d4"), san: "d4".into(), fen_before: String::new(), from_book: false });
        assert_eq!(h.len(), 1);
        assert_eq!(h.get(0).unwrap().san, "d4");
    }

    // ── PHASE 16, Step 3: access to the underlying tree ─────────────────────

    #[test]
    fn test_node_id_at_matches_get() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        let id0 = h.node_id_at(0).unwrap();
        let id1 = h.node_id_at(1).unwrap();
        assert_ne!(id0, id1);
        assert_eq!(h.tree().node(id0).unwrap().record.san, "e4");
        assert_eq!(h.tree().node(id1).unwrap().record.san, "e5");
        assert!(h.node_id_at(2).is_none());
    }

    #[test]
    fn test_node_id_at_empty_history_is_none() {
        let h = History::new();
        assert!(h.node_id_at(0).is_none());
    }

    #[test]
    fn test_last_node_id_tracks_tip() {
        let mut h = History::new();
        assert!(h.last_node_id().is_none());

        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        let after_e4 = h.last_node_id().unwrap();
        assert_eq!(after_e4, h.node_id_at(0).unwrap());

        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        let after_e5 = h.last_node_id().unwrap();
        assert_eq!(after_e5, h.node_id_at(1).unwrap());
        assert_ne!(after_e4, after_e5);

        h.pop();
        assert_eq!(h.last_node_id().unwrap(), after_e4);
    }

    #[test]
    fn test_tree_mut_allows_setting_nag() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        let id = h.last_node_id().unwrap();

        h.tree_mut().node_mut(id).unwrap().nag = Some(crate::game_tree::Nag::Good);

        assert_eq!(h.tree().node(id).unwrap().nag, Some(crate::game_tree::Nag::Good));
    }

    #[test]
    fn test_tree_reflects_pushed_moves() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        assert_eq!(h.tree().len(), 2);
        let root = h.tree().roots()[0];
        assert!(h.tree().is_mainline(root));
    }

    // ── PHASE 16, Step 5: branch_at (variation creation) ────────────────

    #[test]
    fn test_branch_at_out_of_range_returns_none() {
        let mut h = History::new();
        assert!(h.branch_at(0, MoveRecord {
            mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false,
        }).is_none());
        assert!(h.is_empty());
    }

    #[test]
    fn test_branch_at_tip_behaves_like_push() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });

        // ply 0 is the tip: branching here is equivalent to a normal push, the
        // e4 node not yet having any children.
        let id = h.branch_at(0, MoveRecord {
            mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false,
        }).unwrap();

        assert_eq!(h.len(), 2);
        assert_eq!(h.last_node_id(), Some(id));
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "e5"]);
    }

    #[test]
    fn test_branch_at_keeps_discarded_suffix_in_tree() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });
        let old_e5_id  = h.node_id_at(1).unwrap();
        let old_nf3_id = h.node_id_at(2).unwrap();

        // Branch from ply 0 (after 1.e4) with 1...d5 instead of 1...e5:
        // must never remove 1...e5 or 2.Nf3 from the tree (decision 1).
        let new_id = h.branch_at(0, MoveRecord {
            mv: mv("d7", "d5"), san: "d5".into(), fen_before: String::new(), from_book: false,
        }).unwrap();

        // The active line now reflects the variation, not the old continuation.
        assert_eq!(h.len(), 2);
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "d5"]);

        // But the old continuation is indeed still present in the underlying tree.
        assert!(h.tree().node(old_e5_id).is_some(), "1...e5 ne doit pas être supprimé");
        assert!(h.tree().node(old_nf3_id).is_some(), "2.Nf3 ne doit pas être supprimé");
        assert_eq!(h.tree().len(), 4); // e4, e5(variation), Nf3(under the variation), d5

        // The new branch (actively played) becomes children[0] of the
        // e4 node — hence the structural main line — and the old continuation
        // 1...e5 becomes a genuinely real (visible) variation, not the
        // opposite: without this, 1...e5 would remain invisible (never flagged as a
        // variation) and d5 would wrongly be displayed twice as a variation
        // of itself.
        let e4_id = h.node_id_at(0).unwrap();
        let children = &h.tree().node(e4_id).unwrap().children;
        assert_eq!(children[0], new_id);
        assert_eq!(children[1], old_e5_id);
        assert!(h.tree().is_mainline(new_id));
        assert!(!h.tree().is_mainline(old_e5_id));
    }

    #[test]
    fn test_branch_at_then_push_continues_new_branch() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        h.branch_at(0, MoveRecord {
            mv: mv("d7", "d5"), san: "d5".into(), fen_before: String::new(), from_book: false,
        }).unwrap();
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });

        assert_eq!(h.len(), 3);
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "d5", "Nf3"]);
    }

    #[test]
    fn test_branch_at_surfaces_discarded_line_as_variation_block_not_duplicate() {
        // Precisely reproduces the pitfall identified by hand during the
        // implementation of Step 5: without promoting the new
        // branch to `children[0]` (see `branch_at` doc), the discarded
        // continuation (1...e5) would remain invisible and 1...d5 would wrongly display
        // as a "variation" of itself in `build_variation_blocks`.
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });

        h.branch_at(0, MoveRecord {
            mv: mv("d7", "d5"), san: "d5".into(), fen_before: String::new(), from_book: false,
        }).unwrap();

        let blocks = h.tree().build_variation_blocks();
        assert_eq!(blocks.len(), 1, "seule l'ancienne suite (1...e5) doit apparaître en variante");
        assert_eq!(blocks[0].after_ply, 1);
        assert!(blocks[0].text.contains("e5"), "texte inattendu : {}", blocks[0].text);
        assert!(!blocks[0].text.contains("d5"), "d5 ne doit pas apparaître dans sa propre variante");
    }

    #[test]
    fn test_branch_at_middle_of_longer_line() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("b8", "c6"), san: "Nc6".into(), fen_before: String::new(), from_book: false });

        // Variation after white's 2nd move (ply 2 = Nf3): replaces 2...Nc6 with
        // 2...Nf6 as black's reply.
        h.branch_at(2, MoveRecord {
            mv: mv("g8", "f6"), san: "Nf6".into(), fen_before: String::new(), from_book: false,
        }).unwrap();

        assert_eq!(h.len(), 4);
        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "e5", "Nf3", "Nf6"]);
    }

    // ── PHASE 16, Step 6.2: promote_to_mainline / remove_variation ────────

    #[test]
    // Clippy (04/07/2026): `#[allow(similar_names)]` — `nf3_id`/`nc3_id`
    // deliberately mirror the chess notation of the tested moves
    // (Nf3, Nc3), not an accidental mix-up.
    #[allow(clippy::similar_names)]
    fn test_promote_to_mainline_realigns_path_to_promoted_branch() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });
        let nf3_id = h.node_id_at(2).unwrap();

        // 2...replaying from e5 (ply 1) with Nc3: Nf3 is demoted to a variation.
        h.branch_at(1, MoveRecord {
            mv: mv("b1", "c3"), san: "Nc3".into(), fen_before: String::new(), from_book: false,
        }).unwrap();
        assert!(!h.tree().is_mainline(nf3_id), "précondition : Nf3 devenu une variante");

        // Changing our mind: Nf3 must become the active line again.
        assert!(h.promote_to_mainline(nf3_id));

        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "e5", "Nf3"]);
        assert_eq!(h.last_node_id(), Some(nf3_id));
        assert!(h.tree().is_mainline(nf3_id));

        // Nc3 (the old active line) in turn becomes a genuinely real variation.
        let e5_id = h.node_id_at(1).unwrap();
        let nc3_id = h.tree().node(e5_id).unwrap().children[1];
        assert!(!h.tree().is_mainline(nc3_id));
        assert_eq!(h.tree().node(nc3_id).unwrap().record.san, "Nc3");
    }

    #[test]
    fn test_promote_to_mainline_restores_full_recorded_continuation() {
        // The promoted variation itself had an already-recorded continuation
        // (Nc6, Bb5) before being demoted by the `branch_at` below —
        // this continuation must be fully restored into `path`, not
        // just its first move.
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("g1", "f3"), san: "Nf3".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("b8", "c6"), san: "Nc6".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("f1", "b5"), san: "Bb5".into(), fen_before: String::new(), from_book: false });
        let nf3_id = h.node_id_at(2).unwrap();

        h.branch_at(1, MoveRecord {
            mv: mv("b1", "c3"), san: "Nc3".into(), fen_before: String::new(), from_book: false,
        }).unwrap();
        // `branch_at(1, ..)` truncates `path` to 2 elements (e4, e5) then
        // appends the new node (Nc3) to it: the active line therefore goes from 5 to 3
        // moves, not 2 — Nf3/Nc6/Bb5 remain in the tree, just detached from
        // `path` (decision 1, no silent truncation).
        assert_eq!(h.len(), 3, "précondition : la ligne active est raccourcie par le branch_at");

        assert!(h.promote_to_mainline(nf3_id));

        let sans: Vec<&str> = h.san_list().collect();
        assert_eq!(sans, ["e4", "e5", "Nf3", "Nc6", "Bb5"]);
    }

    #[test]
    fn test_promote_to_mainline_unknown_id_returns_false() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });

        assert!(!h.promote_to_mainline(999));
        assert_eq!(h.san_list().collect::<Vec<_>>(), ["e4"], "aucune modification");
    }

    #[test]
    fn test_promote_to_mainline_already_mainline_is_noop_but_returns_true() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        let id = h.node_id_at(0).unwrap();

        assert!(h.promote_to_mainline(id));
        assert_eq!(h.san_list().collect::<Vec<_>>(), ["e4"]);
    }

    #[test]
    fn test_remove_variation_removes_node_and_descendants() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        h.push(MoveRecord { mv: mv("e7", "e5"), san: "e5".into(), fen_before: String::new(), from_book: false });
        let e5_id = h.node_id_at(1).unwrap();
        let nc3_id = h.tree_mut().add_move(Some(e5_id), MoveRecord {
            mv: mv("b1", "c3"), san: "Nc3".into(), fen_before: String::new(), from_book: false,
        }).unwrap();

        assert!(h.remove_variation(nc3_id));

        assert!(h.tree().node(nc3_id).is_none());
        assert_eq!(h.len(), 2, "la ligne active n'est pas affectée par la suppression d'une variante");
    }

    #[test]
    fn test_remove_variation_refuses_when_node_on_path() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });
        let id = h.node_id_at(0).unwrap();

        assert!(!h.remove_variation(id));
        assert!(h.tree().node(id).is_some(), "aucune modification");
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn test_remove_variation_unknown_id_returns_false() {
        let mut h = History::new();
        h.push(MoveRecord { mv: mv("e2", "e4"), san: "e4".into(), fen_before: String::new(), from_book: false });

        assert!(!h.remove_variation(999));
    }
}
