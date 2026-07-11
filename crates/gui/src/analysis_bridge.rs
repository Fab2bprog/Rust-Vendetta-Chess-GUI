//! Bridge between the UCI engine and the Slint interface (Phase 6.7).
//!
//! [`AnalysisBridge`] launches the analysis in a dedicated thread and sends the
//! results to the main Slint thread via [`slint::invoke_from_event_loop`].
//!
//! # Engine discovery
//!
//! The search order is:
//! 1. `VENDETTA_ENGINE` environment variable (absolute path to the binary).
//! 2. `vendetta_chess_motor` binary detected in the system `PATH`.
//!
//! If no engine is found, [`AnalysisBridge::has_engine`] returns
//! `false` and calls to [`AnalysisBridge::start`] are silently
//! ignored.
//!
//! # Life cycle of an analysis
//!
//! ```text
//! start(fen, window)
//!   └─ cancels the previous analysis (atomic flag)
//!   └─ thread → UciEngine::connect → analyze(movetime=1500) → invoke_from_event_loop
//!       └─ set_engine_depth / set_engine_score / set_engine_pv / set_engine_thinking
//! stop() → flag set to true (the thread terminates on the next check)
//! ```

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use uci::{
    engine::{EnginePosition, UciEngine},
    parser::UciScore,
    protocol::GoLimits,
};

use crate::AppWindow;

// ── Score formatting ──────────────────────────────────────────────────────────

/// Converts a [`UciScore`] into a readable string.
///
/// | Score       | Result    |
/// |-------------|-----------|
/// | `+50 cp`    | `"+0.50"` |
/// | `-30 cp`    | `"-0.30"` |
/// | `Mate(3)`   | `"M3"`    |
/// | `Mate(-5)`  | `"-M5"`   |
/// | `None`      | `"—"`     |
// Clippy: `#[allow(cast_precision_loss)]` — a centipawn score fits on
// an `i32` but in practice stays bounded to a few thousand (±5000 pawns at
// the very most); far below the exact-precision limit of an `f32`.
#[allow(clippy::cast_precision_loss)]
fn format_score(score: Option<&UciScore>) -> String {
    match score {
        Some(UciScore::Centipawns(cp))  => format!("{:+.2}", *cp as f32 / 100.0),
        Some(UciScore::Mate(n)) if *n > 0 => format!("M{n}"),
        Some(UciScore::Mate(n))           => format!("-M{}", n.unsigned_abs()),
        // Aspiration bounds: display the approximate value
        Some(UciScore::Lowerbound(cp))  => format!("≥{:+.2}", *cp as f32 / 100.0),
        Some(UciScore::Upperbound(cp))  => format!("≤{:+.2}", *cp as f32 / 100.0),
        None                            => "—".to_owned(),
    }
}

/// Converts a [`UciScore`] into a numeric value, normalized from White's
/// point of view (positive = White's advantage), capped at ±50 pawns (mate).
///
/// `is_white_to_move`: `true` if it is White's turn to play in the position
/// analyzed. The UCI engine always returns the score from the point of view of
/// the player to move; the sign is inverted if it is Black's turn to play.
///
/// Made `pub` (PHASE 82, step 9): reused as-is by `main.rs`
/// for on-demand analysis of a game from the reference database
/// (`analyze-game-detail`), which needs exactly the same conversion
/// as the analysis of an ongoing game, but outside of any `AnalysisBridge`
/// (historical positions, not the live position).
// Clippy: `#[allow(cast_precision_loss)]` — see `format_score`, same
// practical bound (±5000 pawns), safe conversion.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn score_to_f32(score: Option<&UciScore>, is_white_to_move: bool) -> f32 {
    let raw = match score {
        Some(UciScore::Mate(n)) if *n > 0 =>  50.0_f32,
        Some(UciScore::Mate(_))           => -50.0_f32,
        Some(UciScore::Centipawns(cp) | UciScore::Lowerbound(cp) | UciScore::Upperbound(cp)) => *cp as f32 / 100.0,
        None                            => 0.0_f32,
    };
    if is_white_to_move { raw } else { -raw }
}

/// Converts a pawn score (White's perspective) into a [0.0, 1.0] fraction
/// for the evaluation bar via a sigmoid (k = 0.4).
///
/// | Score   | Fraction | Interpretation          |
/// |---------|----------|-------------------------|
/// |  0.0 p  |  0.500   | Equal                   |
/// | +1.0 p  |  0.599   | Slight White advantage  |
/// | +3.0 p  |  0.769   | Significant advantage   |
/// | +5.0 p  |  0.881   | Decisive advantage      |
/// | +50.0 p |  ~1.000  | White mate              |
fn score_to_eval_fraction(score_pawns: f32) -> f32 {
    1.0_f32 / (1.0_f32 + (-score_pawns * 0.4_f32).exp())
}

// ── Engine discovery (dev/test only, not called in production) ────────────────

/// Looks for a UCI engine binary — **development/test use only**.
///
/// **No longer called automatically** in `AnalysisBridge::new_for(true)`.
/// In production, users configure their own UCI engines via
/// Preferences → Engines; `sync_analysis_engine()` in `main.rs` feeds
/// the bridge from this config.
///
/// Can still be used occasionally in dev via `VENDETTA_ENGINE`.
#[allow(dead_code)]
fn discover_engine() -> Option<String> {
    // 1. VENDETTA_ENGINE environment variable
    if let Ok(path) = std::env::var("VENDETTA_ENGINE") {
        if Path::new(&path).is_file() {
            return Some(path);
        }
    }

    // 2. Same directory as the gui executable (target/debug/ or target/release/)
    //    This is the most common case in development with `cargo run -p gui`.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            #[cfg(unix)]
            let candidate = dir.join("vendetta_chess_motor");
            #[cfg(windows)]
            let candidate = dir.join("vendetta_chess_motor.exe");

            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // 3. `vendetta_chess_motor` in the system PATH
    #[cfg(unix)]
    {
        if let Ok(output) = std::process::Command::new("which")
            .arg("vendetta_chess_motor")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if !path.is_empty() && Path::new(&path).is_file() {
                    return Some(path);
                }
            }
        }
    }
    #[cfg(windows)]
    {
        if let Ok(output) = std::process::Command::new("where")
            .arg("vendetta_chess_motor")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if !path.is_empty() && Path::new(&path).is_file() {
                    return Some(path);
                }
            }
        }
    }

    None
}

// ── AnalysisBridge ───────────────────────────────────────────────────────────

/// Manages background UCI analysis for the GUI.
pub struct AnalysisBridge {
    /// Path to the engine binary (`None` if no engine was found).
    engine_path: Option<String>,
    /// Stop flag shared with the currently running analysis thread.
    stop_flag:   Arc<AtomicBool>,
    /// Generation counter: each new `start()` increments this counter.
    /// The thread checks that its generation is still the current one before publishing
    /// results — avoids stale results if `start()` is called twice
    /// in quick succession (C3).
    generation:  Arc<AtomicU64>,
    /// true → feeds engine-pv-lines-white; false → engine-pv-lines-black.
    /// Fixed at construction, never changes.
    for_white:   bool,
}

impl AnalysisBridge {
    /// Creates a new bridge with no engine configured.
    ///
    /// The engine path must be provided via [`set_engine_path`] before
    /// calling [`start`]. In practice, `main.rs` calls
    /// `sync_analysis_engine()` at startup to feed the bridge from
    /// the user configuration (hint engine or first saved engine).
    ///
    /// Auto-discovery of specific binaries (`vendetta_chess_motor`, etc.)
    /// was removed: end users configure their own
    /// UCI engines via Preferences → Engines.
    #[must_use]
    pub fn new_for(for_white: bool) -> Self {
        Self {
            engine_path: None,
            stop_flag:   Arc::new(AtomicBool::new(false)),
            generation:  Arc::new(AtomicU64::new(0)),
            for_white,
        }
    }

    /// `true` if a UCI engine was found.
    #[must_use]
    pub fn has_engine(&self) -> bool {
        self.engine_path.is_some()
    }

    /// Sets (or replaces) the path of the engine used for analysis.
    ///
    /// Allows using an engine manually configured by the user
    /// (via Preferences → Engines) when automatic discovery fails.
    pub fn set_engine_path(&mut self, path: String) {
        self.engine_path = Some(path);
    }

    /// Starts analyzing position `fen` in a dedicated thread.
    ///
    /// Silently cancels any previous analysis. Results are
    /// sent to the Slint window via `invoke_from_event_loop`.
    ///
    /// Has no effect if no engine is configured.
    ///
    /// `is_white_to_move`: side to move in the position (needed to
    /// normalize the score from White's point of view in the
    /// `analysis-completed` callback).
    ///
    /// `multipv_n`: number of `MultiPV` lines to request (1–5). Sends
    /// `setoption name MultiPV value N` before analysis if N > 1.
    // Clippy (04/07/2026): `#[allow(too_many_lines, cast_possible_wrap)]` —
    // function deliberately not split up (explicit choice, cf. user
    // exchange of 04/07/2026): it orchestrates in a single readable block the
    // engine connection, the UCI/MultiPV sending, and the Slint callbacks
    // (thinking/depth/score/pv), all coupled to the same thread generation
    // (C3) — splitting it would multiply the synchronization points between
    // sub-functions for a marginal readability gain, at the cost of a
    // regression risk on a sensitive path (threads + UI). `rank as i32`
    // (MultiPV, rank 1..=5) can never exceed `i32::MAX`.
    #[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
    pub fn start(&mut self, fen: String, window: slint::Weak<AppWindow>, is_white_to_move: bool, multipv_n: u32) {
        // Stop the previous analysis (previous thread of THIS bridge only)
        self.stop_flag.store(true, Ordering::SeqCst);
        let for_white = self.for_white;

        let Some(engine_path) = self.engine_path.clone() else { return };

        let stop = Arc::new(AtomicBool::new(false));
        self.stop_flag = stop.clone();

        // Increment the generation: the thread will carry this value and check
        // that it is still current before publishing results (C3).
        let my_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();

        // Signal "thinking in progress" immediately
        let win = window.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(w) = win.upgrade() {
                w.set_engine_thinking(true);
                w.set_engine_depth("…".into());
                w.set_engine_score("…".into());
                w.set_engine_pv("…".into());
            }
        })
        .ok();

        std::thread::spawn(move || {
        let window_panic = window.clone();
        crate::run_guarded_thread(std::panic::AssertUnwindSafe(move || {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            // Check that we are still the current generation (C3)
            if generation.load(Ordering::SeqCst) != my_gen {
                return;
            }

            // Connection to the engine (timeout reduced to 3 s)
            let Ok(mut engine) = UciEngine::connect_with_timeout(&engine_path, Duration::from_secs(3)) else {
                let win = window.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(w) = win.upgrade() {
                        w.set_engine_thinking(false);
                        w.set_engine_depth("—".into());
                        w.set_engine_score("Moteur injoignable".into());
                        w.set_engine_pv(String::new().into());
                        w.set_eval_bar_visible(false);
                    }
                })
                .ok();
                return;
            };

            if stop.load(Ordering::SeqCst) {
                engine.quit();
                return;
            }

            // MultiPV: send setoption before ucinewgame (already done in connect),
            // but we force it here to make sure it is taken into account.
            let n = multipv_n.max(1);
            if n > 1 {
                let _ = engine.set_option("MultiPV", Some(&n.to_string()));
            } else {
                // Always force to 1 so as not to inherit from a previous session
                let _ = engine.set_option("MultiPV", Some("1"));
            }

            // Analysis with a fixed time limit (1.5 s)
            let position = EnginePosition::from_fen(fen);
            let limits   = GoLimits { movetime: Some(1500), ..GoLimits::default() };

            match engine.analyze(&position, &limits) {
                Ok(result) => {
                    // Check generation before publishing (C3: quick double start)
                    let is_current_gen = generation.load(Ordering::SeqCst) == my_gen;
                    if stop.load(Ordering::SeqCst) || !is_current_gen {
                        slint::invoke_from_event_loop(move || {
                            if let Some(w) = window.upgrade() {
                                w.set_engine_thinking(false);
                                w.set_engine_depth("—".into());
                                w.set_engine_score("—".into());
                                w.set_engine_pv(String::new().into());
                                let empty = std::rc::Rc::new(slint::VecModel::from(vec![])).into();
                                if for_white { w.set_engine_pv_lines_white(empty); }
                                else         { w.set_engine_pv_lines_black(empty); }
                                w.set_eval_bar_visible(false);
                            }
                        }).ok();
                        engine.quit();
                        return;
                    }

                    // ── Build the PvLine entries for the UI ────────────────
                    let mut pv_lines: Vec<crate::PvLine> = Vec::new();
                    for rank in 1..=n {
                        // Take the last info with a non-empty PV (max depth)
                        if let Some(info) = result.multipv_line(rank)
                            .into_iter()
                            .rfind(|i| !i.pv.is_empty())
                        {
                            let depth_s = info.depth.map_or_else(|| "—".to_owned(), |d| d.to_string());
                            let score_s = format_score(info.score.as_ref());
                            let pv_s    = info.pv.join(" ");
                            pv_lines.push(crate::PvLine {
                                rank:  rank as i32,
                                score: score_s.into(),
                                depth: depth_s.into(),
                                pv:    pv_s.into(),
                            });
                        }
                    }

                    // Line-1 score for the evaluation bar
                    let (depth_s, score_s, pv_s, score_f32) =
                        if let Some(pv) = result.principal_variation() {
                            let d  = pv.depth.map_or_else(|| "—".to_owned(), |d| d.to_string());
                            let s  = format_score(pv.score.as_ref());
                            let f  = score_to_f32(pv.score.as_ref(), is_white_to_move);
                            let pv = pv.pv.join(" ");
                            (d, s, pv, f)
                        } else {
                            ("—".to_owned(), "—".to_owned(), String::new(), 0.0_f32)
                        };

                    let eval_frac = score_to_eval_fraction(score_f32);
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window.upgrade() {
                            // Backward compat: scalar properties (line 1)
                            w.set_engine_depth(depth_s.into());
                            w.set_engine_score(score_s.into());
                            w.set_engine_pv(pv_s.into());
                            w.set_engine_thinking(false);
                            // MultiPV: feed the right side's panel
                            let model = std::rc::Rc::new(slint::VecModel::from(pv_lines)).into();
                            if for_white { w.set_engine_pv_lines_white(model); }
                            else         { w.set_engine_pv_lines_black(model); }
                            // Evaluation bar
                            w.set_eval_fraction(eval_frac);
                            w.set_eval_bar_visible(true);
                            w.invoke_analysis_completed(score_f32);
                        }
                    })
                    .ok();
                }
                Err(err) => {
                    eprintln!("[AnalysisBridge] Erreur analyse ({}): {err}", if for_white { "B" } else { "N" });
                    let stderr_lines = engine.last_stderr_lines();
                    if !stderr_lines.is_empty() {
                        eprintln!("[AnalysisBridge] Dernières lignes stderr du moteur :");
                        for line in &stderr_lines {
                            eprintln!("[AnalysisBridge]   {line}");
                        }
                    }
                    slint::invoke_from_event_loop(move || {
                        if let Some(w) = window.upgrade() {
                            w.set_engine_thinking(false);
                            w.set_engine_depth("—".into());
                            w.set_engine_score("—".into());
                            w.set_engine_pv(String::new().into());
                            let empty = std::rc::Rc::new(slint::VecModel::from(vec![])).into();
                            if for_white { w.set_engine_pv_lines_white(empty); }
                            else         { w.set_engine_pv_lines_black(empty); }
                            w.set_eval_bar_visible(false);
                        }
                    })
                    .ok();
                }
            }

            engine.quit();
        }), window_panic, |w| {
            w.set_engine_thinking(false);
            w.set_engine_depth("—".into());
            w.set_engine_score("Erreur interne".into());
            w.set_engine_pv(String::new().into());
            w.set_eval_bar_visible(false);
        });
        });
    }

    /// Stops the ongoing analysis (without blocking).
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

impl Default for AnalysisBridge {
    fn default() -> Self {
        Self::new_for(true)
    }
}

// ── DualAnalysisBridge ──────────────────────────────────────────────────────

/// Two independent UCI bridges: one for White, one for Black.
///
/// Each bridge has its own thread, its own `stop_flag`, and its own
/// generation counter. The analyses of the two sides never interfere with each other.
pub struct DualAnalysisBridge {
    pub white: AnalysisBridge,
    pub black: AnalysisBridge,
}

impl DualAnalysisBridge {
    #[must_use]
    pub fn new() -> Self {
        Self {
            white: AnalysisBridge::new_for(true),
            black: AnalysisBridge::new_for(false),
        }
    }

    /// `true` if an engine is configured (both always share the same one).
    #[must_use]
    pub fn has_engine(&self) -> bool {
        self.white.has_engine()
    }

    /// Sets the engine path on both bridges.
    pub fn set_engine_path(&mut self, path: String) {
        self.white.set_engine_path(path.clone());
        self.black.set_engine_path(path);
    }

    /// Stops both ongoing analyses.
    pub fn stop(&mut self) {
        self.white.stop();
        self.black.stop();
    }

    /// Starts the analysis for the right side.
    pub fn start_for(
        &mut self,
        for_white:        bool,
        fen:              String,
        window:           slint::Weak<AppWindow>,
        is_white_to_move: bool,
        multipv_n:        u32,
    ) {
        if for_white {
            self.white.start(fen, window, is_white_to_move, multipv_n);
        } else {
            self.black.start(fen, window, is_white_to_move, multipv_n);
        }
    }
}

impl Default for DualAnalysisBridge {
    fn default() -> Self { Self::new() }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_score_positive_cp() {
        assert_eq!(format_score(Some(&UciScore::Centipawns(50))), "+0.50");
    }

    #[test]
    fn test_format_score_negative_cp() {
        assert_eq!(format_score(Some(&UciScore::Centipawns(-30))), "-0.30");
    }

    #[test]
    fn test_format_score_zero_cp() {
        assert_eq!(format_score(Some(&UciScore::Centipawns(0))), "+0.00");
    }

    #[test]
    fn test_format_score_mate_positive() {
        assert_eq!(format_score(Some(&UciScore::Mate(3))), "M3");
    }

    #[test]
    fn test_format_score_mate_negative() {
        assert_eq!(format_score(Some(&UciScore::Mate(-5))), "-M5");
    }

    #[test]
    fn test_format_score_none() {
        assert_eq!(format_score(None), "—");
    }

    #[test]
    fn test_new_bridge_has_stop_flag_false() {
        // The stop flag must be false at creation
        let bridge = AnalysisBridge::new_for(true);
        assert!(!bridge.stop_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_stop_sets_flag() {
        let mut bridge = AnalysisBridge::new_for(true);
        bridge.stop();
        assert!(bridge.stop_flag.load(Ordering::SeqCst));
    }
}
