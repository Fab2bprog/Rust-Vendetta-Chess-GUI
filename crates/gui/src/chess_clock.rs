//! Chess clock — pure logic (no Slint dependency).
//!
//! [`ChessClock`] manages the time countdown for both players, across all
//! modes: Fischer, Bronstein, `PerGame`. The tick is called regularly
//! by a Slint timer in `main.rs`; `ChessClock` has no knowledge of
//! the event loop.
//!
//! # Typical life cycle
//!
//! ```
//! use game_config::TimeControl;
//! use gui::chess_clock::ChessClock;
//!
//! let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
//!
//! // Start of the game — White plays first
//! clock.start(true);
//!
//! // ~1 second elapses
//! clock.tick(1_000);
//!
//! // White plays e4 → Fischer increment (+2 s) + Black's turn
//! clock.apply_move_bonus(true);
//! clock.start(false);
//!
//! // 180 s − 1 s + 2 s increment = 181 s = 3:01
//! assert!(clock.is_flagged().is_none());
//! assert_eq!(ChessClock::format(clock.white_ms()), "03:01");
//! ```

use game_config::TimeControl;

// ---------------------------------------------------------------------------
// Struct
// ---------------------------------------------------------------------------

/// Chess clock managing the remaining time of both players.
///
/// Supports Fischer (increment), Bronstein (non-cumulative delay)
/// and `PerGame` (fixed time with no increment) time controls. In `Infinite` /
/// `MoveTime` / `Level` mode, `has_clock()` returns `false` and no countdown
/// is performed.
#[derive(Debug, Clone)]
pub struct ChessClock {
    /// White's remaining time in ms (can be negative after flagging).
    white_ms: i64,
    /// Black's remaining time in ms.
    black_ms: i64,
    /// Active player: `Some(true)` = White, `Some(false)` = Black, `None` = paused.
    active: Option<bool>,
    /// Fischer increment in ms (0 if not applicable).
    increment_ms: i64,
    /// Bronstein delay in ms per move (0 if not applicable).
    delay_ms: i64,
    /// Remaining delay for the current move (Bronstein only).
    delay_remaining_ms: i64,
    /// `false` if the time control does not require a visual clock.
    has_clock: bool,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl ChessClock {
    // ── Constructor ──────────────────────────────────────────────────────────

    /// Creates a new clock initialized from a [`TimeControl`].
    ///
    /// The clock starts paused (`active = None`); call [`start`]
    /// to begin the first player's countdown.
    ///
    /// [`start`]: ChessClock::start
    // Clippy: `#[allow(cast_possible_wrap)]` — the time-control seconds
    // (`initial_secs`/`increment_secs`/`delay_secs`, `u64`) remain in
    // practice chess game durations (a few hours at most),
    // far below the limit of `i64`.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)]
    pub fn new(tc: &TimeControl) -> Self {
        let initial_ms      = tc.initial_secs().unwrap_or(0) as i64 * 1_000;
        let increment_ms    = tc.increment_secs() as i64 * 1_000;
        let delay_ms        = tc.delay_secs()     as i64 * 1_000;

        Self {
            white_ms:           initial_ms,
            black_ms:           initial_ms,
            active:             None,
            increment_ms,
            delay_ms,
            delay_remaining_ms: delay_ms,
            has_clock:          tc.use_player_clock(),
        }
    }

    // ── Control ──────────────────────────────────────────────────────────────

    /// Starts the countdown for player `is_white`.
    ///
    /// Automatically stops the other player and resets the Bronstein delay
    /// for the newly active player.
    pub fn start(&mut self, is_white: bool) {
        self.active             = Some(is_white);
        self.delay_remaining_ms = self.delay_ms;
    }

    /// Pauses the clock (no countdown during pauses, promotions…).
    pub fn stop(&mut self) {
        self.active = None;
    }

    /// Advances the clock by `elapsed_ms` milliseconds.
    ///
    /// - Without a game clock (`has_clock = false`): no effect.
    /// - Paused (`active = None`): no effect.
    /// - Bronstein mode: the delay is consumed first; any
    ///   surplus is deducted from the player's remaining time.
    /// - Fischer / `PerGame` mode: `elapsed_ms` is deducted directly.
    pub fn tick(&mut self, elapsed_ms: i64) {
        if !self.has_clock { return; }
        let Some(is_white) = self.active else { return };

        let mut to_deduct = elapsed_ms;

        // Bronstein: absorb into the delay before touching the time
        if self.delay_remaining_ms > 0 {
            let absorbed = to_deduct.min(self.delay_remaining_ms);
            self.delay_remaining_ms -= absorbed;
            to_deduct -= absorbed;
        }

        if to_deduct > 0 {
            if is_white {
                self.white_ms -= to_deduct;
            } else {
                self.black_ms -= to_deduct;
            }
        }
    }

    /// Adds the Fischer increment to the player who just moved.
    ///
    /// No effect if the time control is not Fischer (increment = 0).
    /// Must be called **after** the move has been validated, **before** `start` for
    /// the opponent.
    pub fn apply_move_bonus(&mut self, is_white: bool) {
        if self.increment_ms == 0 { return; }
        if is_white {
            self.white_ms += self.increment_ms;
        } else {
            self.black_ms += self.increment_ms;
        }
    }

    // ── State ────────────────────────────────────────────────────────────────

    /// Returns the player whose time has run out, or `None` if neither has.
    ///
    /// - `Some(true)`  → White lost on time (White's flag fell)
    /// - `Some(false)` → Black lost on time
    /// - `None`        → neither, or no clock
    #[must_use]
    pub fn is_flagged(&self) -> Option<bool> {
        if !self.has_clock { return None; }
        if self.white_ms <= 0 { return Some(true); }
        if self.black_ms <= 0 { return Some(false); }
        None
    }

    // ── Accessors ────────────────────────────────────────────────────────────

    /// White's remaining time in milliseconds (can be ≤ 0 after flagging).
    #[must_use]
    pub fn white_ms(&self) -> i64 { self.white_ms }

    /// Black's remaining time in milliseconds.
    #[must_use]
    pub fn black_ms(&self) -> i64 { self.black_ms }

    /// Active player: `Some(true)` = White, `Some(false)` = Black, `None` = paused.
    #[must_use]
    pub fn active_player(&self) -> Option<bool> { self.active }

    /// `true` if this time control displays visual clocks.
    #[must_use]
    pub fn has_clock(&self) -> bool { self.has_clock }

    /// Fischer increment in ms (0 if Bronstein/PerGame/no increment).
    ///
    /// Used to fill `winc`/`binc` in the UCI `go` command.
    #[must_use]
    pub fn increment_ms(&self) -> i64 { self.increment_ms }

    /// Replaces the initial times of both players (**handicap** mode).
    ///
    /// Allows initializing asymmetric times (e.g. beginner 15 min vs
    /// coach 3 min). No effect if the clock is of type `Infinite`,
    /// `MoveTime` or `Level` (`has_clock() == false`).
    ///
    /// Must be called **after** [`new`] and **before** [`start`].
    ///
    /// [`new`]: ChessClock::new
    /// [`start`]: ChessClock::start
    pub fn set_initial_times(&mut self, white_ms: i64, black_ms: i64) {
        if self.has_clock {
            self.white_ms = white_ms;
            self.black_ms = black_ms;
        }
    }

    // ── Formatting ───────────────────────────────────────────────────────────

    /// Formats a duration in ms into a readable string.
    ///
    /// - `>= 1 hour` → `"H:MM:SS"`
    /// - `< 1 hour`  → `"MM:SS"`
    /// - Negative    → clamped to 0 before formatting (`"00:00"`)
    // Clippy: `#[allow(cast_sign_loss)]` — `ms.max(0)` guarantees a
    // non-negative value right before the conversion.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn format(ms: i64) -> String {
        let total_secs = (ms.max(0) / 1_000) as u64;
        let hours = total_secs / 3_600;
        let mins  = (total_secs % 3_600) / 60;
        let secs  = total_secs % 60;
        if hours > 0 {
            format!("{hours}:{mins:02}:{secs:02}")
        } else {
            format!("{mins:02}:{secs:02}")
        }
    }

    /// Returns White's formatted time (e.g. `"03:00"`).
    #[must_use]
    pub fn white_display(&self) -> String { Self::format(self.white_ms) }

    /// Returns Black's formatted time.
    #[must_use]
    pub fn black_display(&self) -> String { Self::format(self.black_ms) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Constructor ──────────────────────────────────────────────────────────

    #[test]
    fn test_new_from_fischer_blitz_3_2() {
        let clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        assert_eq!(clock.white_ms(), 180_000);
        assert_eq!(clock.black_ms(), 180_000);
        assert_eq!(clock.increment_ms, 2_000);
        assert_eq!(clock.delay_ms,     0);
        assert!(clock.has_clock());
        assert!(clock.active_player().is_none());
    }

    #[test]
    fn test_new_from_per_game_blitz_5_0() {
        let clock = ChessClock::new(&TimeControl::BLITZ_5_0);
        assert_eq!(clock.white_ms(), 300_000);
        assert_eq!(clock.black_ms(), 300_000);
        assert_eq!(clock.increment_ms, 0);
        assert!(clock.has_clock());
    }

    #[test]
    fn test_new_from_bronstein() {
        let tc = TimeControl::Bronstein { base_secs: 300, delay_secs: 3 };
        let clock = ChessClock::new(&tc);
        assert_eq!(clock.white_ms(), 300_000);
        assert_eq!(clock.delay_ms,   3_000);
        assert_eq!(clock.increment_ms, 0);
        assert!(clock.has_clock());
    }

    #[test]
    fn test_new_from_infinite_no_clock() {
        let clock = ChessClock::new(&TimeControl::Infinite);
        assert!(!clock.has_clock());
        assert_eq!(clock.white_ms(), 0);
        assert_eq!(clock.black_ms(), 0);
    }

    #[test]
    fn test_set_initial_times_asymmetric() {
        // Beginner 15 min vs coach 3 min
        let mut clock = ChessClock::new(&TimeControl::BLITZ_5_0);
        assert_eq!(clock.white_ms(), 300_000); // symmetric 5 min preset
        clock.set_initial_times(900_000, 180_000); // 15 min / 3 min
        assert_eq!(clock.white_ms(), 900_000);
        assert_eq!(clock.black_ms(), 180_000);
    }

    #[test]
    fn test_set_initial_times_no_effect_without_clock() {
        // Without a game clock, set_initial_times has no effect
        let mut clock = ChessClock::new(&TimeControl::Infinite);
        clock.set_initial_times(600_000, 180_000);
        assert_eq!(clock.white_ms(), 0); // unchanged
        assert_eq!(clock.black_ms(), 0);
    }

    #[test]
    fn test_new_from_movetime_no_clock() {
        let clock = ChessClock::new(&TimeControl::MoveTime(5_000));
        assert!(!clock.has_clock());
    }

    #[test]
    fn test_new_from_level_no_clock() {
        let clock = ChessClock::new(&TimeControl::Level(3));
        assert!(!clock.has_clock());
    }

    // ── start / stop ─────────────────────────────────────────────────────────

    #[test]
    fn test_start_sets_active_white() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        assert_eq!(clock.active_player(), Some(true));
    }

    #[test]
    fn test_start_sets_active_black() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(false);
        assert_eq!(clock.active_player(), Some(false));
    }

    #[test]
    fn test_stop_pauses_clock() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        clock.stop();
        assert!(clock.active_player().is_none());
    }

    #[test]
    fn test_start_resets_bronstein_delay() {
        let tc = TimeControl::Bronstein { base_secs: 300, delay_secs: 3 };
        let mut clock = ChessClock::new(&tc);
        // Partially consume the delay
        clock.start(true);
        clock.tick(1_000);
        assert_eq!(clock.delay_remaining_ms, 2_000);
        // start() must reset the delay
        clock.start(false);
        assert_eq!(clock.delay_remaining_ms, 3_000);
    }

    // ── tick — Fischer / PerGame ─────────────────────────────────────────────

    #[test]
    fn test_tick_deducts_from_white() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        clock.tick(500);
        assert_eq!(clock.white_ms(), 179_500);
        assert_eq!(clock.black_ms(), 180_000); // Black untouched
    }

    #[test]
    fn test_tick_deducts_from_black() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(false);
        clock.tick(1_000);
        assert_eq!(clock.black_ms(), 179_000);
        assert_eq!(clock.white_ms(), 180_000);
    }

    #[test]
    fn test_tick_does_nothing_when_stopped() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        // No start → paused
        clock.tick(5_000);
        assert_eq!(clock.white_ms(), 180_000);
        assert_eq!(clock.black_ms(), 180_000);
    }

    #[test]
    fn test_tick_does_nothing_without_player_clock() {
        let mut clock = ChessClock::new(&TimeControl::Infinite);
        clock.start(true);  // start has no real effect without has_clock
        clock.tick(1_000);
        assert_eq!(clock.white_ms(), 0); // stays at 0, no countdown
    }

    #[test]
    fn test_multiple_ticks_accumulate() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_5_0);
        clock.start(true);
        for _ in 0..10 {
            clock.tick(100);
        }
        assert_eq!(clock.white_ms(), 300_000 - 1_000);
    }

    // ── tick — Bronstein ──────────────────────────────────────────────────────

    #[test]
    fn test_bronstein_tick_within_delay_no_deduction() {
        let tc = TimeControl::Bronstein { base_secs: 60, delay_secs: 3 };
        let mut clock = ChessClock::new(&tc);
        clock.start(true);
        // Playing in 2 s → the delay absorbs everything
        clock.tick(2_000);
        assert_eq!(clock.white_ms(), 60_000); // time untouched
        assert_eq!(clock.delay_remaining_ms, 1_000);
    }

    #[test]
    fn test_bronstein_tick_exceeds_delay_deducts_surplus() {
        let tc = TimeControl::Bronstein { base_secs: 60, delay_secs: 3 };
        let mut clock = ChessClock::new(&tc);
        clock.start(true);
        // 5 s of thinking: 3 s absorbed by the delay, 2 s deducted from the time
        clock.tick(5_000);
        assert_eq!(clock.white_ms(), 58_000);
        assert_eq!(clock.delay_remaining_ms, 0);
    }

    #[test]
    fn test_bronstein_exact_delay_no_deduction() {
        let tc = TimeControl::Bronstein { base_secs: 60, delay_secs: 3 };
        let mut clock = ChessClock::new(&tc);
        clock.start(true);
        clock.tick(3_000); // exactly the delay
        assert_eq!(clock.white_ms(), 60_000);
        assert_eq!(clock.delay_remaining_ms, 0);
    }

    #[test]
    fn test_bronstein_multiple_ticks_within_delay() {
        let tc = TimeControl::Bronstein { base_secs: 60, delay_secs: 3 };
        let mut clock = ChessClock::new(&tc);
        clock.start(true);
        clock.tick(1_000);
        clock.tick(1_000);
        // 2 s consumed out of the 3 s delay
        assert_eq!(clock.white_ms(), 60_000);
        assert_eq!(clock.delay_remaining_ms, 1_000);
    }

    // ── apply_move_bonus ─────────────────────────────────────────────────────

    #[test]
    fn test_apply_move_bonus_adds_increment_fischer() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        clock.tick(1_000); // White consumes 1 s
        clock.apply_move_bonus(true);
        // 180 000 - 1 000 + 2 000 = 181 000
        assert_eq!(clock.white_ms(), 181_000);
    }

    #[test]
    fn test_apply_move_bonus_no_increment_per_game() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_5_0);
        clock.start(true);
        clock.tick(1_000);
        clock.apply_move_bonus(true);
        assert_eq!(clock.white_ms(), 299_000); // no bonus
    }

    #[test]
    fn test_apply_move_bonus_only_affects_correct_player() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(false);
        clock.tick(500);
        clock.apply_move_bonus(false);
        // Black: 180 000 - 500 + 2 000 = 181 500
        assert_eq!(clock.black_ms(),  181_500);
        assert_eq!(clock.white_ms(),  180_000); // White untouched
    }

    // ── is_flagged ────────────────────────────────────────────────────────────

    #[test]
    fn test_no_flag_initially() {
        let clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        assert!(clock.is_flagged().is_none());
    }

    #[test]
    fn test_white_flag_when_time_reaches_zero() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        clock.tick(180_000); // consumes all of White's time
        assert_eq!(clock.is_flagged(), Some(true));
    }

    #[test]
    fn test_black_flag_when_time_reaches_zero() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(false);
        clock.tick(180_001); // exceeds Black's time
        assert_eq!(clock.is_flagged(), Some(false));
    }

    #[test]
    fn test_no_flag_when_no_clock() {
        let mut clock = ChessClock::new(&TimeControl::Infinite);
        // white_ms = 0 but has_clock = false → no flag
        assert!(clock.is_flagged().is_none());
        clock.start(true);
        clock.tick(1_000);
        assert!(clock.is_flagged().is_none());
    }

    #[test]
    fn test_white_flagged_takes_priority() {
        // If both times are exhausted (impossible in reality, but defensive check)
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.white_ms = -1;
        clock.black_ms = -1;
        // White is checked first → Some(true)
        assert_eq!(clock.is_flagged(), Some(true));
    }

    // ── format ────────────────────────────────────────────────────────────────

    #[test]
    fn test_format_minutes_seconds() {
        assert_eq!(ChessClock::format(180_000), "03:00");
        assert_eq!(ChessClock::format(90_500),  "01:30");
        assert_eq!(ChessClock::format(0),       "00:00");
        assert_eq!(ChessClock::format(999),     "00:00"); // < 1 s → 00:00
    }

    #[test]
    fn test_format_negative_clamps_to_zero() {
        assert_eq!(ChessClock::format(-5_000), "00:00");
    }

    #[test]
    fn test_format_hours() {
        assert_eq!(ChessClock::format(5_400_000), "1:30:00"); // 90 min
        assert_eq!(ChessClock::format(3_600_000), "1:00:00"); // 60 min
        assert_eq!(ChessClock::format(3_661_000), "1:01:01");
    }

    #[test]
    fn test_format_59_minutes() {
        assert_eq!(ChessClock::format(3_599_000), "59:59");
    }

    // ── white_display / black_display ─────────────────────────────────────────

    #[test]
    fn test_display_helpers() {
        let mut clock = ChessClock::new(&TimeControl::BLITZ_3_2);
        clock.start(true);
        clock.tick(30_000); // -30 s
        assert_eq!(clock.white_display(), "02:30");
        assert_eq!(clock.black_display(), "03:00");
    }

    // ── Realistic scenario: Bullet 1+0 game ──────────────────────────────────

    #[test]
    fn test_bullet_game_scenario() {
        let mut clock = ChessClock::new(&TimeControl::BULLET_1_0);
        assert_eq!(clock.white_ms(), 60_000);

        // White plays e4 in 2 s
        clock.start(true);
        clock.tick(2_000);
        assert_eq!(clock.white_ms(), 58_000);

        // Black plays e5 in 3 s
        clock.start(false);
        clock.tick(3_000);
        assert_eq!(clock.black_ms(), 57_000);

        // No flag
        assert!(clock.is_flagged().is_none());
    }

    #[test]
    fn test_classical_format_hours() {
        let clock = ChessClock::new(&TimeControl::CLASSICAL_90_30);
        assert_eq!(clock.white_display(), "1:30:00");
        assert_eq!(clock.black_display(), "1:30:00");
    }
}
