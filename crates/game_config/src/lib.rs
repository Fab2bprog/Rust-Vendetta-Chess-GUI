//! Game configuration for Vendetta Chess GUI.
//!
//! This crate provides the types representing a complete game configuration
//! (mode, colors, engines, time control) along with JSON serialization /
//! deserialization functions for persistence between sessions.
//!
//! # Example
//!
//! ```rust
//! use game_config::{GameConfig, TimeControl};
//! use game_config::persist;
//!
//! // Blitz 3+2 Human vs Engine game
//! let mut config = GameConfig::human_vs_engine("/usr/local/bin/stockfish");
//! config.time_control = TimeControl::BLITZ_3_2;
//!
//! // Save for the next session
//! persist::save_last_config(&config).ok();
//!
//! // On the next startup:
//! if let Some(last) = persist::load_last_config(game_config::GameMode::HumanVsEngine) {
//!     println!("Reprendre avec {:?}", last.mode);
//! }
//! ```

pub mod persist;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Game mode
// ---------------------------------------------------------------------------

/// Game mode: determines who plays each side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    /// Both sides are humans (no engine player).
    HumanVsHuman,
    /// A human against an engine.
    HumanVsEngine,
    /// Two engines play each other (automatic game).
    EngineVsEngine,
}

// ---------------------------------------------------------------------------
// Human player color
// ---------------------------------------------------------------------------

/// Color chosen by the human player in H vs E mode.
///
/// `Random` is resolved when the game starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HumanColor {
    #[default]
    White,
    Black,
    /// Drawn at random when the game starts.
    Random,
}

// ---------------------------------------------------------------------------
// Time control
// ---------------------------------------------------------------------------

/// Time control of a chess game.
///
/// # Game clock variants (visual clocks)
///
/// - [`Fischer`]   — base + increment per move (modern standard: 3+2, 15+10, 90+30…)
/// - [`Bronstein`] — base + non-cumulative delay (if the player moves before the delay
///   elapses, the unused time is not recovered)
/// - [`PerGame`]   — fixed total time per player, with no increment at all (e.g. blitz 5+0)
///
/// # Engine variants (no visual clock)
///
/// - [`MoveTime`]  — fixed time per move in milliseconds, sent via `go movetime N`
/// - [`Level`]     — predefined level 1→5 converted to movetime (internal engine use)
/// - [`Infinite`]  — infinite thinking until a manual `stop`
///
/// [`Fischer`]:   TimeControl::Fischer
/// [`Bronstein`]: TimeControl::Bronstein
/// [`PerGame`]:   TimeControl::PerGame
/// [`MoveTime`]:  TimeControl::MoveTime
/// [`Level`]:     TimeControl::Level
/// [`Infinite`]:  TimeControl::Infinite
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeControl {
    // ── Game clocks ───────────────────────────────────────────────────

    /// Fischer clock: base time + increment added after every move.
    ///
    /// Example: `{ base_secs: 180, increment_secs: 2 }` = Blitz 3+2
    Fischer {
        /// Initial time per player in seconds.
        base_secs: u64,
        /// Seconds added to the counter after each move played.
        increment_secs: u64,
    },

    /// Bronstein clock: delay before the clock starts counting down.
    ///
    /// If the player moves within the delay, the unused time **is not**
    /// added (unlike Fischer). If the player takes longer than the delay,
    /// only the excess is deducted from the remaining time.
    Bronstein {
        /// Initial time per player in seconds.
        base_secs: u64,
        /// Delay in seconds (non-cumulative).
        delay_secs: u64,
    },

    /// Fixed total time per player, with no increment or delay.
    ///
    /// Example: `{ secs: 300 }` = 5 minutes per player (Blitz 5+0)
    PerGame {
        /// Total time per player in seconds.
        secs: u64,
    },

    // ── Engine use (no visual clock) ────────────────────────────────

    /// Fixed time per move in milliseconds → sent via `go movetime N`.
    MoveTime(u64),

    /// Predefined level (1 = very fast … 5 = very strong).
    ///
    /// Converted to movetime via [`TimeControl::level_movetime_ms`].
    Level(u8),

    /// Infinite: the engine thinks until a manual `stop`.
    Infinite,
}

// ── Standard presets ─────────────────────────────────────────────────────────

impl TimeControl {
    /// Bullet 1+0 — 1 minute per player, no increment.
    pub const BULLET_1_0: Self = Self::PerGame { secs: 60 };

    /// Bullet 2+1 — 2 minutes + 1 second per move.
    pub const BULLET_2_1: Self = Self::Fischer { base_secs: 120, increment_secs: 1 };

    /// Blitz 3+2 — 3 minutes + 2 seconds per move (the most played online).
    pub const BLITZ_3_2: Self = Self::Fischer { base_secs: 180, increment_secs: 2 };

    /// Blitz 5+0 — 5 minutes per player, no increment.
    pub const BLITZ_5_0: Self = Self::PerGame { secs: 300 };

    /// Rapid 10+5 — 10 minutes + 5 seconds per move.
    pub const RAPID_10_5: Self = Self::Fischer { base_secs: 600, increment_secs: 5 };

    /// Rapid 15+10 — 15 minutes + 10 seconds per move.
    pub const RAPID_15_10: Self = Self::Fischer { base_secs: 900, increment_secs: 10 };

    /// FIDE Classical — 90 minutes + 30 seconds per move.
    pub const CLASSICAL_90_30: Self = Self::Fischer { base_secs: 5_400, increment_secs: 30 };
}

// ── Methods ─────────────────────────────────────────────────────────────────

impl TimeControl {
    // ── Game clock methods ─────────────────────────────────────────

    /// Initial time per player in seconds.
    ///
    /// Returns `Some(n)` for variants with a visual clock
    /// (`Fischer`, `Bronstein`, `PerGame`), `None` for the others.
    #[must_use]
    pub fn initial_secs(self) -> Option<u64> {
        match self {
            Self::Fischer { base_secs, .. } | Self::Bronstein { base_secs, .. } => Some(base_secs),
            Self::PerGame { secs }                                              => Some(secs),
            _                                                                    => None,
        }
    }

    /// Fischer increment in seconds added after every move.
    ///
    /// Returns `0` for all variants without an increment.
    #[must_use]
    pub fn increment_secs(self) -> u64 {
        match self {
            Self::Fischer { increment_secs, .. } => increment_secs,
            _                                     => 0,
        }
    }

    /// Bronstein delay in seconds.
    ///
    /// Returns `0` for all variants without a delay.
    #[must_use]
    pub fn delay_secs(self) -> u64 {
        match self {
            Self::Bronstein { delay_secs, .. } => delay_secs,
            _                                   => 0,
        }
    }

    /// `true` if this time control requires a visual clock.
    ///
    /// True for `Fischer`, `Bronstein`, `PerGame`.
    /// False for `MoveTime`, `Level`, `Infinite`.
    #[must_use]
    pub fn use_player_clock(self) -> bool {
        self.initial_secs().is_some()
    }

    // ── UCI engine methods ───────────────────────────────────────────────────

    /// Level → movetime (ms) conversion table, sorted by increasing duration.
    ///
    /// | Level  | Movetime   |
    /// |--------|------------|
    /// | 1      | 100 ms     | 0.1 s
    /// | 2      | 500 ms     | 0.5 s
    /// | 3      | 1 000 ms   | 1 s
    /// | 4      | 1 500 ms   | 1.5 s
    /// | 5      | 3 000 ms   | 3 s
    /// | 6      | 5 000 ms   | 5 s
    /// | 7      | 15 000 ms  | 15 s
    /// | 8      | 30 000 ms  | 30 s
    /// | 9      | 60 000 ms  | 1 min
    /// | 10     | 120 000 ms | 2 min
    /// | 11     | 180 000 ms | 3 min
    /// | 12+    | 300 000 ms | 5 min
    #[must_use]
    pub fn level_movetime_ms(level: u8) -> u64 {
        match level {
            1 => 100,
            2 => 500,
            3 => 1_000,
            4 => 1_500,
            5 => 3_000,
            6 => 5_000,
            7 => 15_000,
            8 => 30_000,
            9 => 60_000,
            10 => 120_000,
            11 => 180_000,
            _ => 300_000,
        }
    }

    /// Returns the effective movetime in ms for the `go movetime N` command.
    ///
    /// - `MoveTime(n)` → `Some(n)`
    /// - `Level(n)`    → `Some(level_movetime_ms(n))`
    /// - Others        → `None` (time is managed by the clock or is infinite)
    #[must_use]
    pub fn movetime_ms(self) -> Option<u64> {
        match self {
            Self::MoveTime(ms) => Some(ms),
            Self::Level(lvl)   => Some(Self::level_movetime_ms(lvl)),
            _                   => None,
        }
    }

    /// `true` if the time control is infinite.
    #[must_use]
    pub fn is_infinite(self) -> bool {
        matches!(self, Self::Infinite)
    }

    /// Short representation for display (e.g. "3+2", "5+0", "Infini").
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Fischer   { base_secs, increment_secs } =>
                format!("{}+{}", base_secs / 60, increment_secs),
            Self::Bronstein { base_secs, delay_secs } =>
                format!("{}+{}d", base_secs / 60, delay_secs),
            Self::PerGame   { secs }   => format!("{}+0", secs / 60),
            Self::MoveTime(ms)         => format!("{} s/coup", ms / 1_000),
            Self::Level(n)             => format!("Niveau {n}"),
            Self::Infinite             => "Infini".into(),
        }
    }
}

impl Default for TimeControl {
    /// Default for the game clock: Blitz 5+0.
    ///
    /// Note: `EngineSettings.time_control` uses its own default (`Level(3)`)
    /// via `default_engine_tc()`.
    fn default() -> Self { Self::BLITZ_5_0 }
}

/// Default for `EngineSettings.time_control`: Level 3 (1,000 ms/move).
fn default_engine_tc() -> TimeControl { TimeControl::Level(3) }

/// Default for `GameConfig.time_control`: Infinite (no clock displayed).
///
/// Allows JSON backward compatibility: old configs without this field do
/// not trigger a clock.
fn default_game_tc() -> TimeControl { TimeControl::Infinite }

// ---------------------------------------------------------------------------
// Engine configuration
// ---------------------------------------------------------------------------

/// UCI settings of an engine for a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSettings {
    /// Absolute path to the engine binary.
    pub path: String,
    /// UCI options to apply (name → value as a string).
    ///
    /// Only options **different from the engine's default value** should
    /// be stored here.
    pub options: HashMap<String, String>,
    /// Thinking time per move for this engine (UCI `go movetime`).
    ///
    /// In game clock mode (`GameConfig.time_control` = Fischer/Bronstein/PerGame),
    /// this field is ignored: the engine receives `go wtime … btime … winc … binc …`.
    #[serde(default = "default_engine_tc")]
    pub time_control: TimeControl,
}

impl EngineSettings {
    /// Creates a minimal configuration for the `path` binary.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path:         path.into(),
            options:      HashMap::new(),
            time_control: default_engine_tc(),
        }
    }

    /// Sets a UCI option (overwrites if already present).
    pub fn set_option(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.options.insert(name.into(), value.into());
    }

    /// Removes a UCI option (the engine will use its default value).
    pub fn remove_option(&mut self, name: &str) {
        self.options.remove(name);
    }

    /// Returns the value of a UCI option, or `None` if not set.
    #[must_use]
    pub fn get_option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    /// Returns `true` if at least one UCI option has been set.
    #[must_use]
    pub fn has_custom_options(&self) -> bool {
        !self.options.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Game configuration
// ---------------------------------------------------------------------------

/// Full configuration of a Vendetta Chess game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    /// Game mode.
    pub mode: GameMode,
    /// Color chosen by the human player (ignored in E vs E).
    pub human_color: HumanColor,
    /// Engine playing White (`None` if a human plays White).
    pub white_engine: Option<EngineSettings>,
    /// Engine playing Black (`None` if a human plays Black).
    pub black_engine: Option<EngineSettings>,
    /// Time control of the game (clock shared by both players).
    ///
    /// `Infinite` = no visual clock (analysis or beginner mode).
    /// `Fischer` / `Bronstein` / `PerGame` = countdown clocks displayed during play.
    ///
    /// For engines, the clock passes `wtime/btime/winc/binc` via UCI.
    /// If `Infinite` or `MoveTime`, the engine receives `go movetime N` from
    /// `EngineSettings.time_control`.
    #[serde(default = "default_game_tc")]
    pub time_control: TimeControl,
    /// White's initial time in seconds — overrides the `time_control` preset.
    ///
    /// `None` → both players get the same time from the preset (balanced game).
    /// `Some(N)` → White starts with N seconds (**handicap** mode).
    /// Has no effect if `time_control` is `Infinite`, `MoveTime` or `Level`.
    #[serde(default)]
    pub white_time_secs_override: Option<u64>,
    /// Black's initial time in seconds — overrides the `time_control` preset.
    ///
    /// `None` → symmetric preset value. `Some(N)` → N seconds (**handicap** mode).
    #[serde(default)]
    pub black_time_secs_override: Option<u64>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            mode:         GameMode::HumanVsEngine,
            human_color:  HumanColor::White,
            white_engine: None,
            black_engine: None,
            time_control: default_game_tc(),
            white_time_secs_override: None,
            black_time_secs_override: None,
        }
    }
}

impl GameConfig {
    // -----------------------------------------------------------------------
    // Convenience constructors
    // -----------------------------------------------------------------------

    /// H vs H game — no engine.
    #[must_use]
    pub fn human_vs_human() -> Self {
        Self {
            mode:         GameMode::HumanVsHuman,
            human_color:  HumanColor::White,
            white_engine: None,
            black_engine: None,
            time_control: default_game_tc(),
            white_time_secs_override: None,
            black_time_secs_override: None,
        }
    }

    /// H vs E game — the human plays White, the engine plays Black.
    #[must_use]
    pub fn human_vs_engine(engine_path: impl Into<String>) -> Self {
        Self {
            mode:         GameMode::HumanVsEngine,
            human_color:  HumanColor::White,
            white_engine: None,
            black_engine: Some(EngineSettings::new(engine_path)),
            time_control: default_game_tc(),
            white_time_secs_override: None,
            black_time_secs_override: None,
        }
    }

    /// H vs E game — the human plays Black, the engine plays White.
    #[must_use]
    pub fn human_vs_engine_as_black(engine_path: impl Into<String>) -> Self {
        Self {
            mode:         GameMode::HumanVsEngine,
            human_color:  HumanColor::Black,
            white_engine: Some(EngineSettings::new(engine_path)),
            black_engine: None,
            time_control: default_game_tc(),
            white_time_secs_override: None,
            black_time_secs_override: None,
        }
    }

    /// E vs E game — two engines play each other.
    #[must_use]
    pub fn engine_vs_engine(
        white_path: impl Into<String>,
        black_path: impl Into<String>,
    ) -> Self {
        Self {
            mode:         GameMode::EngineVsEngine,
            human_color:  HumanColor::White,
            white_engine: Some(EngineSettings::new(white_path)),
            black_engine: Some(EngineSettings::new(black_path)),
            time_control: default_game_tc(),
            white_time_secs_override: None,
            black_time_secs_override: None,
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Settings of the engine playing White.
    #[must_use]
    pub fn white_engine(&self) -> Option<&EngineSettings> {
        self.white_engine.as_ref()
    }

    /// Settings of the engine playing Black.
    #[must_use]
    pub fn black_engine(&self) -> Option<&EngineSettings> {
        self.black_engine.as_ref()
    }

    /// Settings of the engine for the given side.
    #[must_use]
    pub fn engine_for(&self, is_white: bool) -> Option<&EngineSettings> {
        if is_white { self.white_engine() } else { self.black_engine() }
    }

    /// `true` if an engine plays White.
    #[must_use]
    pub fn engine_plays_white(&self) -> bool {
        self.white_engine.is_some()
    }

    /// `true` if an engine plays Black.
    #[must_use]
    pub fn engine_plays_black(&self) -> bool {
        self.black_engine.is_some()
    }

    /// `true` if an engine plays the `is_white` side.
    #[must_use]
    pub fn engine_plays(&self, is_white: bool) -> bool {
        if is_white { self.engine_plays_white() } else { self.engine_plays_black() }
    }

    /// Resolves `HumanColor::Random`: returns `true` = White, `false` = Black.
    #[must_use]
    pub fn resolve_human_is_white(&self) -> bool {
        match self.human_color {
            HumanColor::White  => true,
            HumanColor::Black  => false,
            HumanColor::Random => roll_random_is_white(),
        }
    }
}

/// Flips a coin for a color (`true` = White, `false` = Black), with no
/// external dependency (no `rand` crate in this workspace).
///
/// PHASE 66: extracted from [`GameConfig::resolve_human_is_white`] so it can
/// be called directly from the GUI side (`crates/gui/src/main.rs`, callback
/// `on_setup_start`) — the draw must happen EXACTLY ONCE before building the
/// `GameConfig`, so that the same value is used both for engine/human
/// assignment and for the board's orientation.
///
/// Fix (same PHASE, reported by the user after 11 consecutive attempts all
/// won by White): the first version used the parity of
/// `SystemTime::now()`'s nanoseconds. On some platforms, the system clock's
/// *effective* resolution is actually on the order of a microsecond (or
/// coarser), meaning the reported nanoseconds are ALWAYS a multiple of
/// 1000 — hence ALWAYS even — which biased the draw 100% toward "White",
/// regardless of the actual time of the click. We now use the standard
/// library's `RandomState` (the same mechanism used to randomly salt
/// `HashMap`s against collision attacks): its keys are derived from a real
/// operating-system entropy source, and differ on every call. We also mix
/// in the clock (`SipHash` washes out any low-bit bias) for extra
/// robustness, without ever relying solely on either one.
#[must_use]
pub fn roll_random_is_white() -> bool {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    hasher.write_u128(now_nanos);
    hasher.finish().is_multiple_of(2)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── GameMode ─────────────────────────────────────────────────────────────

    #[test]
    fn test_game_mode_variants() {
        assert_ne!(GameMode::HumanVsHuman, GameMode::HumanVsEngine);
        assert_ne!(GameMode::HumanVsEngine, GameMode::EngineVsEngine);
    }

    // ── TimeControl — engine variants (backward compat) ────────────────────────

    #[test]
    fn test_time_control_movetime() {
        assert_eq!(TimeControl::MoveTime(500).movetime_ms(), Some(500));
    }

    #[test]
    fn test_time_control_level_movetime() {
        assert_eq!(TimeControl::level_movetime_ms(1), 100);
        assert_eq!(TimeControl::level_movetime_ms(2), 500);
        assert_eq!(TimeControl::level_movetime_ms(3), 1_000);
        assert_eq!(TimeControl::level_movetime_ms(4), 1_500);
        assert_eq!(TimeControl::level_movetime_ms(5), 3_000);
        assert_eq!(TimeControl::level_movetime_ms(6), 5_000);
        assert_eq!(TimeControl::level_movetime_ms(7), 15_000);
        assert_eq!(TimeControl::level_movetime_ms(8), 30_000);
        assert_eq!(TimeControl::level_movetime_ms(9), 60_000);
        assert_eq!(TimeControl::level_movetime_ms(10), 120_000);
        assert_eq!(TimeControl::level_movetime_ms(11), 180_000);
        assert_eq!(TimeControl::level_movetime_ms(12), 300_000);
        assert_eq!(TimeControl::level_movetime_ms(99), 300_000);
    }

    #[test]
    fn test_time_control_level_to_movetime() {
        assert_eq!(TimeControl::Level(3).movetime_ms(), Some(1_000));
        assert_eq!(TimeControl::Level(7).movetime_ms(), Some(15_000));
    }

    #[test]
    fn test_time_control_infinite() {
        assert_eq!(TimeControl::Infinite.movetime_ms(), None);
        assert!(TimeControl::Infinite.is_infinite());
        assert!(!TimeControl::MoveTime(100).is_infinite());
    }

    // ── TimeControl — new clock variants ───────────────────────────

    #[test]
    fn test_fischer_initial_secs() {
        let tc = TimeControl::Fischer { base_secs: 180, increment_secs: 2 };
        assert_eq!(tc.initial_secs(),    Some(180));
        assert_eq!(tc.increment_secs(),  2);
        assert_eq!(tc.delay_secs(),      0);
        assert!(tc.use_player_clock());
        assert_eq!(tc.movetime_ms(),     None);
        assert!(!tc.is_infinite());
    }

    #[test]
    fn test_bronstein_initial_secs() {
        let tc = TimeControl::Bronstein { base_secs: 300, delay_secs: 3 };
        assert_eq!(tc.initial_secs(),    Some(300));
        assert_eq!(tc.increment_secs(),  0);
        assert_eq!(tc.delay_secs(),      3);
        assert!(tc.use_player_clock());
        assert_eq!(tc.movetime_ms(),     None);
    }

    #[test]
    fn test_per_game_initial_secs() {
        let tc = TimeControl::PerGame { secs: 300 };
        assert_eq!(tc.initial_secs(),    Some(300));
        assert_eq!(tc.increment_secs(),  0);
        assert_eq!(tc.delay_secs(),      0);
        assert!(tc.use_player_clock());
        assert_eq!(tc.movetime_ms(),     None);
    }

    #[test]
    fn test_movetime_no_player_clock() {
        assert!(!TimeControl::MoveTime(2000).use_player_clock());
        assert_eq!(TimeControl::MoveTime(2000).initial_secs(), None);
    }

    #[test]
    fn test_level_no_player_clock() {
        assert!(!TimeControl::Level(3).use_player_clock());
        assert_eq!(TimeControl::Level(3).initial_secs(), None);
    }

    #[test]
    fn test_infinite_no_player_clock() {
        assert!(!TimeControl::Infinite.use_player_clock());
        assert_eq!(TimeControl::Infinite.initial_secs(), None);
    }

    // ── Presets ──────────────────────────────────────────────────────────────

    #[test]
    fn test_preset_bullet_1_0() {
        assert_eq!(TimeControl::BULLET_1_0.initial_secs(), Some(60));
        assert_eq!(TimeControl::BULLET_1_0.increment_secs(), 0);
    }

    #[test]
    fn test_preset_bullet_2_1() {
        assert_eq!(TimeControl::BULLET_2_1.initial_secs(), Some(120));
        assert_eq!(TimeControl::BULLET_2_1.increment_secs(), 1);
    }

    #[test]
    fn test_preset_blitz_3_2() {
        assert_eq!(TimeControl::BLITZ_3_2.initial_secs(), Some(180));
        assert_eq!(TimeControl::BLITZ_3_2.increment_secs(), 2);
    }

    #[test]
    fn test_preset_blitz_5_0() {
        assert_eq!(TimeControl::BLITZ_5_0.initial_secs(), Some(300));
        assert_eq!(TimeControl::BLITZ_5_0.increment_secs(), 0);
    }

    #[test]
    fn test_preset_rapid_10_5() {
        assert_eq!(TimeControl::RAPID_10_5.initial_secs(), Some(600));
        assert_eq!(TimeControl::RAPID_10_5.increment_secs(), 5);
    }

    #[test]
    fn test_preset_rapid_15_10() {
        assert_eq!(TimeControl::RAPID_15_10.initial_secs(), Some(900));
        assert_eq!(TimeControl::RAPID_15_10.increment_secs(), 10);
    }

    #[test]
    fn test_preset_classical() {
        assert_eq!(TimeControl::CLASSICAL_90_30.initial_secs(), Some(5_400));
        assert_eq!(TimeControl::CLASSICAL_90_30.increment_secs(), 30);
    }

    // ── label() ──────────────────────────────────────────────────────────────

    #[test]
    fn test_label_fischer() {
        assert_eq!(TimeControl::BLITZ_3_2.label(),  "3+2");
        assert_eq!(TimeControl::RAPID_10_5.label(), "10+5");
    }

    #[test]
    fn test_label_per_game() {
        assert_eq!(TimeControl::BLITZ_5_0.label(),   "5+0");
        assert_eq!(TimeControl::BULLET_1_0.label(),  "1+0");
    }

    #[test]
    fn test_label_bronstein() {
        let tc = TimeControl::Bronstein { base_secs: 300, delay_secs: 3 };
        assert_eq!(tc.label(), "5+3d");
    }

    #[test]
    fn test_label_movetime() {
        assert_eq!(TimeControl::MoveTime(3_000).label(), "3 s/coup");
    }

    #[test]
    fn test_label_level() {
        assert_eq!(TimeControl::Level(3).label(), "Niveau 3");
    }

    #[test]
    fn test_label_infinite() {
        assert_eq!(TimeControl::Infinite.label(), "Infini");
    }

    // ── default() ────────────────────────────────────────────────────────────

    #[test]
    fn test_time_control_default_is_blitz_5_0() {
        assert_eq!(TimeControl::default(), TimeControl::BLITZ_5_0);
        assert!(TimeControl::default().use_player_clock());
    }

    #[test]
    fn test_engine_settings_default_tc_is_level3() {
        // The default for EngineSettings stays Level(3) for UCI compatibility
        let s = EngineSettings::new("/bin/engine");
        assert_eq!(s.time_control, TimeControl::Level(3));
    }

    #[test]
    fn test_game_config_default_tc_is_infinite() {
        // Backward compat: old configs without time_control → no clock
        let c = GameConfig::default();
        assert_eq!(c.time_control, TimeControl::Infinite);
        assert!(!c.time_control.use_player_clock());
    }

    // ── EngineSettings ───────────────────────────────────────────────────────

    #[test]
    fn test_engine_settings_new() {
        let s = EngineSettings::new("/usr/bin/stockfish");
        assert_eq!(s.path, "/usr/bin/stockfish");
        assert!(s.options.is_empty());
        assert_eq!(s.time_control, TimeControl::Level(3));
    }

    #[test]
    fn test_engine_settings_set_option() {
        let mut s = EngineSettings::new("/bin/engine");
        s.set_option("Hash", "256");
        s.set_option("Threads", "8");
        assert_eq!(s.get_option("Hash"), Some("256"));
        assert_eq!(s.get_option("Threads"), Some("8"));
        assert!(s.has_custom_options());
    }

    #[test]
    fn test_engine_settings_remove_option() {
        let mut s = EngineSettings::new("/bin/engine");
        s.set_option("Hash", "256");
        s.remove_option("Hash");
        assert_eq!(s.get_option("Hash"), None);
        assert!(!s.has_custom_options());
    }

    #[test]
    fn test_engine_settings_get_missing_option() {
        let s = EngineSettings::new("/bin/engine");
        assert_eq!(s.get_option("Threads"), None);
    }

    // ── GameConfig constructors ──────────────────────────────────────────────

    #[test]
    fn test_human_vs_human() {
        let c = GameConfig::human_vs_human();
        assert_eq!(c.mode, GameMode::HumanVsHuman);
        assert!(c.white_engine.is_none());
        assert!(c.black_engine.is_none());
        assert!(!c.engine_plays_white());
        assert!(!c.engine_plays_black());
    }

    #[test]
    fn test_human_vs_engine_human_white() {
        let c = GameConfig::human_vs_engine("/bin/engine");
        assert_eq!(c.mode,        GameMode::HumanVsEngine);
        assert_eq!(c.human_color, HumanColor::White);
        assert!(c.white_engine.is_none());
        assert!(c.black_engine.is_some());
        assert!(!c.engine_plays_white());
        assert!(c.engine_plays_black());
        assert_eq!(c.black_engine().unwrap().path, "/bin/engine");
    }

    #[test]
    fn test_human_vs_engine_human_black() {
        let c = GameConfig::human_vs_engine_as_black("/bin/engine");
        assert_eq!(c.human_color, HumanColor::Black);
        assert!(c.white_engine.is_some());
        assert!(c.black_engine.is_none());
        assert!(c.engine_plays_white());
        assert!(!c.engine_plays_black());
    }

    #[test]
    fn test_engine_vs_engine() {
        let c = GameConfig::engine_vs_engine("/bin/sf", "/bin/lc0");
        assert_eq!(c.mode, GameMode::EngineVsEngine);
        assert!(c.engine_plays_white());
        assert!(c.engine_plays_black());
        assert_eq!(c.white_engine().unwrap().path, "/bin/sf");
        assert_eq!(c.black_engine().unwrap().path, "/bin/lc0");
    }

    #[test]
    fn test_engine_for() {
        let c = GameConfig::engine_vs_engine("/bin/white", "/bin/black");
        assert_eq!(c.engine_for(true).unwrap().path,  "/bin/white");
        assert_eq!(c.engine_for(false).unwrap().path, "/bin/black");
    }

    #[test]
    fn test_engine_plays() {
        let c = GameConfig::human_vs_engine("/bin/engine");
        assert!(!c.engine_plays(true));   // human = White
        assert!(c.engine_plays(false));   // engine = Black
    }

    #[test]
    fn test_resolve_human_is_white() {
        assert!(GameConfig::human_vs_engine("/bin/e").resolve_human_is_white());
        assert!(!GameConfig::human_vs_engine_as_black("/bin/e").resolve_human_is_white());
    }

    #[test]
    fn test_resolve_random_returns_bool() {
        let mut c = GameConfig::human_vs_engine("/bin/e");
        c.human_color = HumanColor::Random;
        let _ = c.resolve_human_is_white();
    }

    /// PHASE 66 non-regression: the very first version of
    /// `roll_random_is_white` (parity of `SystemTime::now`'s nanoseconds)
    /// ALWAYS returned `true` on some platforms (coarse effective clock
    /// resolution → nanoseconds always multiples of 1000, hence always
    /// even) — reported by the user after 11 consecutive attempts all won
    /// by White. This test draws a large number of times and requires
    /// seeing BOTH outcomes appear, to prevent a future implementation
    /// from falling back into the same bias without the test catching it.
    #[test]
    fn test_roll_random_is_white_not_biased() {
        let mut seen_white = false;
        let mut seen_black = false;
        for _ in 0..200 {
            if roll_random_is_white() { seen_white = true; } else { seen_black = true; }
            if seen_white && seen_black { break; }
        }
        assert!(seen_white, "roll_random_is_white() n'a jamais tiré Blancs sur 200 essais");
        assert!(seen_black, "roll_random_is_white() n'a jamais tiré Noirs sur 200 essais");
    }

    // ── GameConfig.time_control ───────────────────────────────────────────────

    #[test]
    fn test_game_config_with_fischer() {
        let mut c = GameConfig::human_vs_human();
        c.time_control = TimeControl::BLITZ_3_2;
        assert!(c.time_control.use_player_clock());
        assert_eq!(c.time_control.initial_secs(), Some(180));
        assert_eq!(c.time_control.increment_secs(), 2);
    }

    #[test]
    fn test_game_config_with_per_game() {
        let mut c = GameConfig::human_vs_human();
        c.time_control = TimeControl::BLITZ_5_0;
        assert!(c.time_control.use_player_clock());
        assert_eq!(c.time_control.initial_secs(), Some(300));
    }

    // ── JSON serialization ────────────────────────────────────────────────────

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut config = GameConfig::human_vs_engine("/usr/bin/stockfish");
        config.time_control = TimeControl::BLITZ_3_2;
        if let Some(e) = config.black_engine.as_mut() {
            e.set_option("Hash", "128");
            e.set_option("Threads", "4");
            e.time_control = TimeControl::MoveTime(3000);
        }

        let json   = serde_json::to_string_pretty(&config).unwrap();
        let loaded: GameConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.mode,         GameMode::HumanVsEngine);
        assert_eq!(loaded.human_color,  HumanColor::White);
        assert_eq!(loaded.time_control, TimeControl::BLITZ_3_2);
        assert!(loaded.white_engine.is_none());

        let engine = loaded.black_engine.unwrap();
        assert_eq!(engine.path, "/usr/bin/stockfish");
        assert_eq!(engine.get_option("Hash"),    Some("128"));
        assert_eq!(engine.get_option("Threads"), Some("4"));
        assert_eq!(engine.time_control, TimeControl::MoveTime(3000));
    }

    #[test]
    fn test_serialize_engine_vs_engine() {
        let config = GameConfig::engine_vs_engine("/bin/sf", "/bin/lc0");
        let json   = serde_json::to_string(&config).unwrap();
        let loaded: GameConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.mode, GameMode::EngineVsEngine);
        assert_eq!(loaded.white_engine.unwrap().path, "/bin/sf");
        assert_eq!(loaded.black_engine.unwrap().path, "/bin/lc0");
    }

    #[test]
    fn test_serialize_time_control_all_variants() {
        let variants = [
            TimeControl::MoveTime(1500),
            TimeControl::Level(3),
            TimeControl::Infinite,
            TimeControl::PerGame   { secs: 300 },
            TimeControl::Fischer   { base_secs: 180, increment_secs: 2 },
            TimeControl::Bronstein { base_secs: 300, delay_secs: 3 },
            TimeControl::CLASSICAL_90_30,
        ];
        for tc in variants {
            let json:   String      = serde_json::to_string(&tc).unwrap();
            let loaded: TimeControl = serde_json::from_str(&json).unwrap();
            assert_eq!(loaded, tc, "round-trip échoué pour {tc:?}");
        }
    }

    #[test]
    fn test_old_json_without_time_control_defaults_to_infinite() {
        // Backward compat: old JSON without the time_control field
        let json = r#"{"mode":"HumanVsEngine","human_color":"White","white_engine":null,"black_engine":null}"#;
        let loaded: GameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.time_control, TimeControl::Infinite);
    }

    #[test]
    fn test_old_engine_settings_without_time_control_defaults_to_level3() {
        // Backward compat: old EngineSettings JSON without time_control
        let json = r#"{"path":"/bin/sf","options":{}}"#;
        let loaded: EngineSettings = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.time_control, TimeControl::Level(3));
    }
}
