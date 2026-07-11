//! State machine for the UCI protocol.
//!
//! ## States
//!
//! ```text
//!  ┌─────────┐  send_uci()   ┌──────────────┐  on_uciok()  ┌───────┐
//!  │  Idle   │ ────────────► │ Initializing │ ───────────► │ Ready │
//!  └─────────┘               └──────────────┘              └───┬───┘
//!                                                              │  ▲
//!                            start_thinking() │                │  │ on_bestmove()
//!                                             ▼                │  │
//!                                        ┌─────────┐           │  │
//!                                        │Thinking │ ──────────┘  │
//!                                        └─────────┘              │
//!                                             │    stop() ────────┘
//! ```
//!
//! Invalid transitions return [`TransitionError`] without modifying the state.

use crate::parser::UciMessage;

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

/// Current state of the UCI protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UciState {
    /// Process launched, handshake not yet started.
    Idle,
    /// `uci` sent, waiting for `uciok`.
    Initializing,
    /// Engine ready to receive commands.
    Ready,
    /// Search in progress, waiting for `bestmove`.
    Thinking,
}

// ---------------------------------------------------------------------------
// Transition errors
// ---------------------------------------------------------------------------

/// Error on an invalid transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub from:    UciState,
    pub action:  &'static str,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Transition invalide depuis {:?} : {}", self.from, self.action)
    }
}

impl std::error::Error for TransitionError {}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// UCI state machine.
///
/// Maintains the current state and validates each transition. The `on_*`
/// methods are called when a message from the engine is received; the
/// `send_*` / `start_*` / `stop` methods correspond to GUI actions.
#[derive(Debug, Clone)]
pub struct UciStateMachine {
    state: UciState,
}

impl UciStateMachine {
    /// Creates a new state machine in state [`UciState::Idle`].
    #[must_use]
    pub fn new() -> Self {
        Self { state: UciState::Idle }
    }

    /// Current state.
    #[must_use]
    pub fn state(&self) -> UciState {
        self.state
    }

    // -----------------------------------------------------------------------
    // GUI actions → transitions
    // -----------------------------------------------------------------------

    /// The GUI sends `uci`: Idle → Initializing.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the current state is not `Idle`.
    pub fn send_uci(&mut self) -> Result<(), TransitionError> {
        self.require(UciState::Idle, "send_uci")?;
        self.state = UciState::Initializing;
        Ok(())
    }

    /// The GUI starts a search: Ready → Thinking.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the current state is not `Ready`.
    pub fn start_thinking(&mut self) -> Result<(), TransitionError> {
        self.require(UciState::Ready, "start_thinking")?;
        self.state = UciState::Thinking;
        Ok(())
    }

    /// The GUI sends `stop`: Thinking → Ready.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the current state is not `Thinking`.
    pub fn stop(&mut self) -> Result<(), TransitionError> {
        self.require(UciState::Thinking, "stop")?;
        self.state = UciState::Ready;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Engine events → transitions
    // -----------------------------------------------------------------------

    /// Received `uciok`: Initializing → Ready.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the current state is not `Initializing`.
    pub fn on_uciok(&mut self) -> Result<(), TransitionError> {
        self.require(UciState::Initializing, "on_uciok")?;
        self.state = UciState::Ready;
        Ok(())
    }

    /// Received `bestmove`: Thinking → Ready.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the current state is not `Thinking`.
    pub fn on_bestmove(&mut self) -> Result<(), TransitionError> {
        self.require(UciState::Thinking, "on_bestmove")?;
        self.state = UciState::Ready;
        Ok(())
    }

    /// Processes a UCI message received from the engine and performs the
    /// associated transition.
    ///
    /// Only `uciok` and `bestmove` trigger transitions; other
    /// messages (`info`, `id`, `option`, `readyok`, `Unknown`) are ignored
    /// by the state machine (they are handled elsewhere).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if the transition is invalid.
    pub fn handle_message(&mut self, msg: &UciMessage) -> Result<(), TransitionError> {
        match msg {
            UciMessage::UciOk             => self.on_uciok(),
            UciMessage::BestMove { .. }   => self.on_bestmove(),
            // All other messages do not change the state
            _ => Ok(()),
        }
    }

    // -----------------------------------------------------------------------
    // Predicates
    // -----------------------------------------------------------------------

    /// Returns `true` if the engine is ready to receive commands.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state == UciState::Ready
    }

    /// Returns `true` if a search is in progress.
    #[must_use]
    pub fn is_thinking(&self) -> bool {
        self.state == UciState::Thinking
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn require(&self, expected: UciState, action: &'static str) -> Result<(), TransitionError> {
        if self.state == expected {
            Ok(())
        } else {
            Err(TransitionError { from: self.state, action })
        }
    }
}

impl Default for UciStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::UciMessage;

    fn init_to_ready() -> UciStateMachine {
        let mut sm = UciStateMachine::new();
        sm.send_uci().unwrap();
        sm.on_uciok().unwrap();
        sm
    }

    // --- Initial state ---

    #[test]
    fn test_initial_state() {
        let sm = UciStateMachine::new();
        assert_eq!(sm.state(), UciState::Idle);
        assert!(!sm.is_ready());
        assert!(!sm.is_thinking());
    }

    // --- Idle → Initializing ---

    #[test]
    fn test_send_uci_from_idle() {
        let mut sm = UciStateMachine::new();
        sm.send_uci().unwrap();
        assert_eq!(sm.state(), UciState::Initializing);
    }

    #[test]
    fn test_send_uci_invalid_from_initializing() {
        let mut sm = UciStateMachine::new();
        sm.send_uci().unwrap();
        let err = sm.send_uci().unwrap_err();
        assert_eq!(err.from, UciState::Initializing);
    }

    #[test]
    fn test_send_uci_invalid_from_ready() {
        let mut sm = init_to_ready();
        let err = sm.send_uci().unwrap_err();
        assert_eq!(err.from, UciState::Ready);
    }

    // --- Initializing → Ready ---

    #[test]
    fn test_on_uciok() {
        let mut sm = UciStateMachine::new();
        sm.send_uci().unwrap();
        sm.on_uciok().unwrap();
        assert_eq!(sm.state(), UciState::Ready);
        assert!(sm.is_ready());
    }

    #[test]
    fn test_on_uciok_invalid_from_idle() {
        let mut sm = UciStateMachine::new();
        let err = sm.on_uciok().unwrap_err();
        assert_eq!(err.from, UciState::Idle);
    }

    // --- Ready → Thinking ---

    #[test]
    fn test_start_thinking() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        assert_eq!(sm.state(), UciState::Thinking);
        assert!(sm.is_thinking());
        assert!(!sm.is_ready());
    }

    #[test]
    fn test_start_thinking_invalid_from_idle() {
        let mut sm = UciStateMachine::new();
        let err = sm.start_thinking().unwrap_err();
        assert_eq!(err.from, UciState::Idle);
    }

    #[test]
    fn test_start_thinking_invalid_from_thinking() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        let err = sm.start_thinking().unwrap_err();
        assert_eq!(err.from, UciState::Thinking);
    }

    // --- Thinking → Ready (stop) ---

    #[test]
    fn test_stop_from_thinking() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        sm.stop().unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    #[test]
    fn test_stop_invalid_from_ready() {
        let mut sm = init_to_ready();
        let err = sm.stop().unwrap_err();
        assert_eq!(err.from, UciState::Ready);
    }

    // --- Thinking → Ready (bestmove) ---

    #[test]
    fn test_on_bestmove() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        sm.on_bestmove().unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    #[test]
    fn test_on_bestmove_invalid_from_ready() {
        let mut sm = init_to_ready();
        let err = sm.on_bestmove().unwrap_err();
        assert_eq!(err.from, UciState::Ready);
    }

    // --- Full cycle ---

    #[test]
    fn test_full_cycle() {
        let mut sm = UciStateMachine::new();

        // Initialization
        assert_eq!(sm.state(), UciState::Idle);
        sm.send_uci().unwrap();
        assert_eq!(sm.state(), UciState::Initializing);
        sm.on_uciok().unwrap();
        assert_eq!(sm.state(), UciState::Ready);

        // First search → bestmove
        sm.start_thinking().unwrap();
        assert_eq!(sm.state(), UciState::Thinking);
        sm.on_bestmove().unwrap();
        assert_eq!(sm.state(), UciState::Ready);

        // Second search → stop
        sm.start_thinking().unwrap();
        sm.stop().unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    // --- handle_message ---

    #[test]
    fn test_handle_uciok_message() {
        let mut sm = UciStateMachine::new();
        sm.send_uci().unwrap();
        sm.handle_message(&UciMessage::UciOk).unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    #[test]
    fn test_handle_bestmove_message() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        let msg = UciMessage::BestMove { mv: "e2e4".into(), ponder: None };
        sm.handle_message(&msg).unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    #[test]
    fn test_handle_info_message_no_transition() {
        let mut sm = init_to_ready();
        sm.start_thinking().unwrap();
        // Info does not change the state
        let msg = UciMessage::ReadyOk;
        sm.handle_message(&msg).unwrap();
        assert_eq!(sm.state(), UciState::Thinking);
    }

    #[test]
    fn test_handle_unknown_message_no_transition() {
        let mut sm = init_to_ready();
        let msg = UciMessage::Unknown("blah".into());
        sm.handle_message(&msg).unwrap();
        assert_eq!(sm.state(), UciState::Ready);
    }

    // --- TransitionError display ---

    #[test]
    fn test_transition_error_display() {
        let err = TransitionError { from: UciState::Idle, action: "start_thinking" };
        let s = err.to_string();
        assert!(s.contains("Idle"));
        assert!(s.contains("start_thinking"));
    }
}
