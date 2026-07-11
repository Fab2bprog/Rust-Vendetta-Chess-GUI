//! Engine-player bridge for the H vs M and M vs M modes.
//!
//! [`GameBridge`] manages one or two engine game threads, one per color.
//! Each thread owns its own [`UciEngine`], sends `setoption` +
//! `ucinewgame` at the start of the game, then loops over the FENs received via
//! an [`mpsc`] channel.
//!
//! # Life cycle
//!
//! ```text
//! GameBridge::new()
//!   └─ init(config, window)
//!         ├─ (H vs H) → no thread
//!         ├─ (H vs M) → 1 thread for the opposing engine
//!         └─ (M vs M) → 2 threads (one per color)
//!
//! trigger_if_engine_turn(is_white, fen)
//!   └─ sends the FEN to the right thread → engine.analyze() → invoke_from_event_loop
//!         └─ window.invoke_engine_move_ready(bestmove)
//!
//! reset()   → closes the channels → the threads quit cleanly
//! ```

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

use game_config::{GameConfig, GameMode, TimeControl};
use uci::{
    engine::{EnginePosition, UciEngine},
    protocol::GoLimits,
};

use crate::AppWindow;

// ── Engine-player thread ────────────────────────────────────────────────────

/// Internal loop of an engine-player thread.
///
/// Connects to the engine once, applies the UCI options, sends
/// `ucinewgame`, then loops over the `(FEN, Option<GoLimits>)` messages received.
/// - `Some(limits)` → active game clock, uses `wtime`/`btime`/`winc`/`binc`
/// - `None`         → no clock, uses the fallback movetime (Level/MoveTime)
///
/// Terminates cleanly when the channel is closed.
// Clippy (04/07/2026): `#[allow(too_many_arguments)]` — background
// thread function that deliberately groups all the necessary context
// (engine connection, UCI protocol, Slint callbacks, generation
// synchronization); same justification as `db::tournament_repo` (previous
// audit). The parameters are passed by reference by the caller (the
// thread captured by `move ||` keeps ownership for the entire
// synchronous call to this function).
#[allow(clippy::too_many_arguments)]
fn engine_player_thread(
    path:              &str,
    options:           &[(String, String)],
    movetime_fallback: u64,
    is_white:          bool,
    receiver:          &mpsc::Receiver<(String, Option<GoLimits>)>,
    window:            &slint::Weak<AppWindow>,
    generation:        &Arc<AtomicU64>,
    my_generation:     u64,
) {
    // ── Connection (5 s timeout) ─────────────────────────────────────────────
    let mut engine = match UciEngine::connect_with_timeout(path, Duration::from_secs(5)) {
        Ok(e)  => e,
        Err(err) => {
            eprintln!("[GameBridge] Connexion échouée ({}): {}", if is_white { "B" } else { "N" }, err);
            let win = window.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(w) = win.upgrade() {
                    w.set_engine_playing(false);
                    w.set_engine_thinking(false);
                }
            }).ok();
            return;
        }
    };

    // ── UCI options + ucinewgame ──────────────────────────────────────────────
    let side = if is_white { "Blancs" } else { "Noirs" };
    if options.is_empty() {
        eprintln!("[GameBridge] {side} : aucune option UCI personnalisée");
    }
    for (name, value) in options {
        eprintln!("[GameBridge] {side} : setoption name {name} value {value}");
        let _ = engine.set_option(name, Some(value.as_str()));
    }
    let _ = engine.new_game();

    // ── Game loop ────────────────────────────────────────────────────────────
    while let Ok((fen, limits_opt)) = receiver.recv() {
        // Signal "thinking in progress" on the Slint thread
        let win = window.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(w) = win.upgrade() {
                w.set_engine_playing(true);
                w.set_engine_thinking(true);
                w.set_engine_depth("…".into());
                w.set_engine_score("…".into());
                w.set_engine_pv("…".into());
            }
        }).ok();

        // UCI limits: active clock → wtime/btime/winc/binc; otherwise fixed movetime
        let limits = limits_opt.unwrap_or_else(|| GoLimits {
            movetime: Some(movetime_fallback),
            ..GoLimits::default()
        });

        // Analysis (blocking until bestmove)
        let position = EnginePosition::from_fen(fen);

        match engine.analyze(&position, &limits) {
            Ok(result) => {
                // If a reset()/init() happened during the calculation (new
                // game, undo, tournament chaining to the next game...),
                // the current generation no longer matches this
                // thread's: the computed move refers to a position/game
                // that is now stale and must absolutely not be applied.
                if generation.load(Ordering::SeqCst) != my_generation {
                    eprintln!(
                        "[GameBridge] Coup ignoré ({}) — génération périmée (reset entre-temps)",
                        if is_white { "B" } else { "N" }
                    );
                    continue;
                }

                let best_move = result.best_move.clone();
                let win = window.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(w) = win.upgrade() {
                        w.set_engine_playing(false);
                        w.set_engine_thinking(false);
                        w.invoke_engine_move_ready(best_move.into());
                    }
                }).ok();
            }
            Err(err) => {
                eprintln!("[GameBridge] Erreur analyse ({}): {}", if is_white { "B" } else { "N" }, err);
                let stderr_lines = engine.last_stderr_lines();
                if !stderr_lines.is_empty() {
                    eprintln!("[GameBridge] Dernières lignes stderr du moteur :");
                    for line in &stderr_lines {
                        eprintln!("[GameBridge]   {line}");
                    }
                }
                let win = window.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(w) = win.upgrade() {
                        w.set_engine_playing(false);
                        w.set_engine_thinking(false);
                    }
                }).ok();
            }
        }
    }

    // Channel closed → quit cleanly
    engine.quit();
}

// ── GameBridge ───────────────────────────────────────────────────────────────

/// Manages the engine-player(s) of a game.
///
/// One `SyncSender<String>` per engaged color: sending a FEN triggers
/// that side's engine to think.
/// Closing the sender (via [`reset`]) cleanly ends the thread.
///
/// [`reset`]: GameBridge::reset
// Clippy: `#[allow(type_complexity)]` avoided via an alias — channel + optional
// move sent to the engine-player thread (FEN, UCI limits if applicable).
type EngineChannel = mpsc::SyncSender<(String, Option<GoLimits>)>;
/// Result of creating an engine-player thread: `None` if the engine's
/// path is empty (engine not configured for this side).
type SpawnedEngine = Option<(EngineChannel, JoinHandle<()>)>;

pub struct GameBridge {
    /// Channel to the white-engine thread (`None` if White is human).
    sender_w: Option<EngineChannel>,
    /// Channel to the black-engine thread (`None` if Black is human).
    sender_b: Option<EngineChannel>,
    /// Handle of the white-engine thread, joined on the next `reset()`.
    handle_w: Option<JoinHandle<()>>,
    /// Handle of the black-engine thread, joined on the next `reset()`.
    handle_b: Option<JoinHandle<()>>,
    /// Generation counter: incremented on every `reset()`/`init()`.
    /// Lets engine-player threads detect that a `bestmove` computed
    /// before a reset is now stale and must not be applied.
    generation: Arc<AtomicU64>,
}

impl GameBridge {
    /// Creates a bridge with no active engine.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sender_w: None,
            sender_b: None,
            handle_w: None,
            handle_b: None,
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Stops the running threads, then initializes new engines according to `config`.
    ///
    /// No effect if the mode is [`GameMode::HumanVsHuman`].
    pub fn init(&mut self, config: &GameConfig, window: &slint::Weak<AppWindow>) {
        self.reset();

        // Current generation: captured by each newly created thread
        // so it can later be compared against `self.generation`.
        let current_generation = self.generation.load(Ordering::SeqCst);

        // Spawns an engine-player thread and returns (sender, handle).
        // Returns None if the path is empty (engine not configured).
        let spawn_side = move |path: &str, options: &std::collections::HashMap<String, String>,
                               tc: TimeControl, is_white: bool,
                               win: slint::Weak<AppWindow>,
                               generation: Arc<AtomicU64>|
                               -> SpawnedEngine {
            if path.is_empty() {
                eprintln!("[GameBridge] Chemin moteur vide, thread non démarré.");
                return None;
            }
            // PHASE 73 — `path` (coming from `EngineSettings.path`, itself
            // derived from the RELATIVE path chosen in the New Game
            // assistant, or possibly absolute for a tournament/an old
            // saved game) is only resolved to absolute here, right before actually
            // launching the engine process — the only place that needs it.
            // `to_absolute_path` handles both cases (relative or already absolute).
            let path = app_paths::to_absolute_path(path).to_string_lossy().into_owned();
            let opts: Vec<(String, String)> = options.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            // Fallback movetime: used only when no active game clock is running.
            // Level → predefined table, MoveTime → direct value, Infinite/Fischer/etc. → 30 s
            let movetime_fallback = tc.movetime_ms().unwrap_or(30_000);

            let (tx, rx) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
            let handle = std::thread::spawn(move || {
                engine_player_thread(
                    &path, &opts, movetime_fallback, is_white, &rx, &win,
                    &generation, current_generation,
                );
            });
            Some((tx, handle))
        };

        match config.mode {
            GameMode::HumanVsHuman => {}

            GameMode::HumanVsEngine => {
                // H vs M: determine which side the engine plays.
                // Creates the thread for the side that has an `EngineSettings`.
                if let Some(e) = &config.white_engine {
                    if let Some((tx, h)) = spawn_side(
                        &e.path, &e.options, e.time_control, true, window.clone(), self.generation.clone(),
                    ) {
                        self.sender_w = Some(tx);
                        self.handle_w = Some(h);
                    }
                }
                if let Some(e) = &config.black_engine {
                    if let Some((tx, h)) = spawn_side(
                        &e.path, &e.options, e.time_control, false, window.clone(), self.generation.clone(),
                    ) {
                        self.sender_b = Some(tx);
                        self.handle_b = Some(h);
                    }
                }
            }

            GameMode::EngineVsEngine => {
                if let Some(e) = &config.white_engine {
                    if let Some((tx, h)) = spawn_side(
                        &e.path, &e.options, e.time_control, true, window.clone(), self.generation.clone(),
                    ) {
                        self.sender_w = Some(tx);
                        self.handle_w = Some(h);
                    }
                }
                if let Some(e) = &config.black_engine {
                    if let Some((tx, h)) = spawn_side(
                        &e.path, &e.options, e.time_control, false, window.clone(), self.generation.clone(),
                    ) {
                        self.sender_b = Some(tx);
                        self.handle_b = Some(h);
                    }
                }
            }
        }
    }

    /// Closes the channels (the threads quit cleanly after the move in progress),
    /// then ensures their actual termination (`join`) is waited for — on a
    /// dedicated cleanup thread rather than blocking the caller.
    ///
    /// Incrementing the generation immediately invalidates any result
    /// that a still-computing thread would return after this point —
    /// this alone is what guarantees correctness (see
    /// `engine_player_thread`'s generation check right before it would
    /// otherwise publish a stale `bestmove`); joining the old threads is
    /// a resource-hygiene measure (guaranteeing the old engine
    /// *processes* are fully terminated — not just "will terminate
    /// eventually" — before the executable might be reused by a new
    /// game), not a correctness requirement.
    ///
    /// Robustness audit 11/07/2026, finding 3.4: the `join` used to run
    /// synchronously here, on the caller's thread — which, for every
    /// current call site, is the UI thread (`main.rs`: "New game"/"Undo"/
    /// ... all call `reset()` directly from a Slint callback). If both
    /// engines were mid-`analyze()` when the user triggered a reset
    /// (M vs M, both engines thinking), the UI froze until *both*
    /// `quit()` calls completed — bounded by `uci`'s internal timeouts
    /// (a few seconds in the worst case, historically), but finding 2.4
    /// now deliberately lets `analyze()` run far longer at a slow game
    /// clock, which would have made this freeze worse. Since correctness
    /// never depended on the join completing before `reset()` returns
    /// (see above), the join is now performed by a short-lived cleanup
    /// thread instead, and `reset()` itself returns immediately.
    pub fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.sender_w = None; // drop → RecvError in the thread → engine.quit()
        self.sender_b = None;

        let handle_w = self.handle_w.take();
        let handle_b = self.handle_b.take();
        if handle_w.is_some() || handle_b.is_some() {
            std::thread::spawn(move || {
                if let Some(h) = handle_w {
                    let _ = h.join();
                }
                if let Some(h) = handle_b {
                    let _ = h.join();
                }
            });
        }
    }

    /// `true` if White is played by an engine.
    #[must_use]
    pub fn has_white_engine(&self) -> bool { self.sender_w.is_some() }

    /// `true` if Black is played by an engine.
    #[must_use]
    pub fn has_black_engine(&self) -> bool { self.sender_b.is_some() }

    /// Asks the white engine to play from position `fen`.
    ///
    /// `limits_opt`: `Some(limits)` if a game clock is active (→ wtime/btime/winc/binc);
    /// `None` → the thread uses its fallback movetime (Level/MoveTime).
    /// No effect if White is human or if the channel is saturated.
    pub fn request_white_move(&self, fen: String, limits_opt: Option<GoLimits>) {
        if let Some(tx) = &self.sender_w {
            if let Err(e) = tx.try_send((fen, limits_opt)) {
                eprintln!("[GameBridge] FEN ignoré (blancs) — canal saturé ou fermé : {e}");
            }
        }
    }

    /// Asks the black engine to play from position `fen`.
    pub fn request_black_move(&self, fen: String, limits_opt: Option<GoLimits>) {
        if let Some(tx) = &self.sender_b {
            if let Err(e) = tx.try_send((fen, limits_opt)) {
                eprintln!("[GameBridge] FEN ignoré (noirs) — canal saturé ou fermé : {e}");
            }
        }
    }

    /// If it's an engine's turn, sends it the FEN + limits and returns `true`.
    ///
    /// `is_white_turn`: `true` if it's White's turn to play.
    /// `limits_opt`: `Some` if a game clock is active, `None` for fixed movetime.
    #[must_use]
    pub fn trigger_if_engine_turn(&self, is_white_turn: bool, fen: String, limits_opt: Option<GoLimits>) -> bool {
        if is_white_turn && self.has_white_engine() {
            self.request_white_move(fen, limits_opt);
            true
        } else if !is_white_turn && self.has_black_engine() {
            self.request_black_move(fen, limits_opt);
            true
        } else {
            false
        }
    }
}

impl Default for GameBridge {
    fn default() -> Self { Self::new() }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_no_engines() {
        let b = GameBridge::new();
        assert!(!b.has_white_engine());
        assert!(!b.has_black_engine());
    }

    #[test]
    fn test_trigger_no_engine_returns_false() {
        let b = GameBridge::new();
        assert!(!b.trigger_if_engine_turn(true,  "fen".into(), None));
        assert!(!b.trigger_if_engine_turn(false, "fen".into(), None));
    }

    #[test]
    fn test_reset_clears_senders() {
        let mut b = GameBridge::new();
        let (tx, _rx) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        b.sender_w = Some(tx);
        assert!(b.has_white_engine());
        b.reset();
        assert!(!b.has_white_engine());
    }

    #[test]
    fn test_has_engine_flags_after_manual_assignment() {
        let mut b = GameBridge::new();
        let (tx_w, _rx_w) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        let (tx_b, _rx_b) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        b.sender_w = Some(tx_w);
        b.sender_b = Some(tx_b);
        assert!(b.has_white_engine());
        assert!(b.has_black_engine());
    }

    #[test]
    fn test_trigger_white_engine() {
        let mut b = GameBridge::new();
        let (tx, rx) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        b.sender_w = Some(tx);

        assert!(b.trigger_if_engine_turn(true, "test_fen".into(), None));
        let (received_fen, _limits) = rx.try_recv().unwrap();
        assert_eq!(received_fen, "test_fen");
    }

    #[test]
    fn test_trigger_black_engine() {
        let mut b = GameBridge::new();
        let (tx, rx) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        b.sender_b = Some(tx);

        assert!(b.trigger_if_engine_turn(false, "test_fen".into(), None));
        let (received_fen, _limits) = rx.try_recv().unwrap();
        assert_eq!(received_fen, "test_fen");
    }

    #[test]
    fn test_trigger_white_does_not_trigger_black_engine() {
        // Black engine available but it's White's turn → not triggered
        let mut b = GameBridge::new();
        let (tx, _rx) = mpsc::sync_channel::<(String, Option<GoLimits>)>(1);
        b.sender_b = Some(tx);
        assert!(!b.trigger_if_engine_turn(true, "fen".into(), None));
    }
}
