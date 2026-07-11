//! Encoder for UCI commands sent to the engine (GUI → engine).
//!
//! The UCI protocol defines the following commands:
//!
//! | Command        | Description                                         |
//! |----------------|-----------------------------------------------------|
//! | `uci`          | Starts the UCI handshake                            |
//! | `debug`        | Enables/disables debug mode                         |
//! | `isready`      | Checks that the engine is ready                     |
//! | `setoption`    | Configures an engine option                         |
//! | `ucinewgame`   | Indicates the start of a new game                   |
//! | `position`     | Sets the position (FEN + moves)                     |
//! | `go`           | Starts the search with the given limits             |
//! | `stop`         | Stops the ongoing search                            |
//! | `ponderhit`    | The player played the ponder move                   |
//! | `quit`         | Asks the engine to terminate                        |
//!
//! Each function returns a `String` ready to be sent via stdin.

// ---------------------------------------------------------------------------
// Search limits for `go`
// ---------------------------------------------------------------------------

/// Time/depth limits for the `go` command.
///
/// All fields are optional: only the ones set are included in the
/// command. A `GoLimits::default()` sends `go` with no restriction
/// (infinite search).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoLimits {
    /// Time remaining for White (ms).
    pub wtime:     Option<u64>,
    /// Time remaining for Black (ms).
    pub btime:     Option<u64>,
    /// Increment per move for White (ms).
    pub winc:      Option<u64>,
    /// Increment per move for Black (ms).
    pub binc:      Option<u64>,
    /// Number of moves until the next time control.
    pub movestogo: Option<u32>,
    /// Maximum search depth.
    pub depth:     Option<u32>,
    /// Maximum number of nodes.
    pub nodes:     Option<u64>,
    /// Mate in `n` moves (forced mate search).
    pub mate:      Option<u32>,
    /// Fixed time allocated to this move (ms).
    pub movetime:  Option<u64>,
    /// Infinite search (stops only on `stop`).
    pub infinite:  bool,
    /// Restrict the search to these moves (UCI: `searchmoves e2e4 d2d4`).
    pub searchmoves: Vec<String>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// `uci` — starts the UCI handshake.
#[must_use]
pub fn cmd_uci() -> String {
    "uci".to_owned()
}

/// `debug on` / `debug off`.
#[must_use]
pub fn cmd_debug(on: bool) -> String {
    if on { "debug on".to_owned() } else { "debug off".to_owned() }
}

/// `isready` — checks that the engine is ready.
#[must_use]
pub fn cmd_isready() -> String {
    "isready".to_owned()
}

/// `setoption name <name> value <value>` — configures an option.
///
/// If `value` is `None` (UCI buttons), only the name is sent.
#[must_use]
pub fn cmd_setoption(name: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => format!("setoption name {name} value {v}"),
        None    => format!("setoption name {name}"),
    }
}

/// `ucinewgame` — new game.
#[must_use]
pub fn cmd_ucinewgame() -> String {
    "ucinewgame".to_owned()
}

/// `position startpos [moves <m1> <m2> …]`
///
/// Sets the engine to the starting position, optionally applying
/// a sequence of UCI moves.
#[must_use]
pub fn cmd_position_startpos(moves: &[&str]) -> String {
    if moves.is_empty() {
        "position startpos".to_owned()
    } else {
        format!("position startpos moves {}", moves.join(" "))
    }
}

/// `position fen <fen> [moves <m1> <m2> …]`
///
/// Sets the engine to the given FEN position, optionally applying
/// a sequence of UCI moves.
#[must_use]
pub fn cmd_position_fen(fen: &str, moves: &[&str]) -> String {
    if moves.is_empty() {
        format!("position fen {fen}")
    } else {
        format!("position fen {fen} moves {}", moves.join(" "))
    }
}

/// `go [options…]` — starts the search.
///
/// Limits that are not set are omitted from the command.
#[must_use]
pub fn cmd_go(limits: &GoLimits) -> String {
    let mut parts = vec!["go".to_owned()];

    if !limits.searchmoves.is_empty() {
        parts.push(format!("searchmoves {}", limits.searchmoves.join(" ")));
    }
    if limits.infinite {
        parts.push("infinite".to_owned());
        return parts.join(" ");
    }
    if let Some(v) = limits.wtime     { parts.push(format!("wtime {v}")); }
    if let Some(v) = limits.btime     { parts.push(format!("btime {v}")); }
    if let Some(v) = limits.winc      { parts.push(format!("winc {v}"));  }
    if let Some(v) = limits.binc      { parts.push(format!("binc {v}"));  }
    if let Some(v) = limits.movestogo { parts.push(format!("movestogo {v}")); }
    if let Some(v) = limits.depth     { parts.push(format!("depth {v}")); }
    if let Some(v) = limits.nodes     { parts.push(format!("nodes {v}")); }
    if let Some(v) = limits.mate      { parts.push(format!("mate {v}"));  }
    if let Some(v) = limits.movetime  { parts.push(format!("movetime {v}")); }

    parts.join(" ")
}

/// `stop` — stops the search.
#[must_use]
pub fn cmd_stop() -> String {
    "stop".to_owned()
}

/// `ponderhit` — the player played the ponder move.
#[must_use]
pub fn cmd_ponderhit() -> String {
    "ponderhit".to_owned()
}

/// `quit` — asks the engine to terminate.
#[must_use]
pub fn cmd_quit() -> String {
    "quit".to_owned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Simple commands ---

    #[test]
    fn test_cmd_uci() {
        assert_eq!(cmd_uci(), "uci");
    }

    #[test]
    fn test_cmd_isready() {
        assert_eq!(cmd_isready(), "isready");
    }

    #[test]
    fn test_cmd_ucinewgame() {
        assert_eq!(cmd_ucinewgame(), "ucinewgame");
    }

    #[test]
    fn test_cmd_stop() {
        assert_eq!(cmd_stop(), "stop");
    }

    #[test]
    fn test_cmd_ponderhit() {
        assert_eq!(cmd_ponderhit(), "ponderhit");
    }

    #[test]
    fn test_cmd_quit() {
        assert_eq!(cmd_quit(), "quit");
    }

    // --- debug ---

    #[test]
    fn test_cmd_debug_on() {
        assert_eq!(cmd_debug(true), "debug on");
    }

    #[test]
    fn test_cmd_debug_off() {
        assert_eq!(cmd_debug(false), "debug off");
    }

    // --- setoption ---

    #[test]
    fn test_cmd_setoption_with_value() {
        assert_eq!(
            cmd_setoption("Hash", Some("128")),
            "setoption name Hash value 128"
        );
    }

    #[test]
    fn test_cmd_setoption_no_value() {
        assert_eq!(
            cmd_setoption("Clear Hash", None),
            "setoption name Clear Hash"
        );
    }

    #[test]
    fn test_cmd_setoption_multiword_name() {
        assert_eq!(
            cmd_setoption("Skill Level", Some("20")),
            "setoption name Skill Level value 20"
        );
    }

    // --- position ---

    #[test]
    fn test_cmd_position_startpos_no_moves() {
        assert_eq!(cmd_position_startpos(&[]), "position startpos");
    }

    #[test]
    fn test_cmd_position_startpos_with_moves() {
        assert_eq!(
            cmd_position_startpos(&["e2e4", "e7e5", "g1f3"]),
            "position startpos moves e2e4 e7e5 g1f3"
        );
    }

    #[test]
    fn test_cmd_position_fen_no_moves() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert_eq!(
            cmd_position_fen(fen, &[]),
            format!("position fen {fen}")
        );
    }

    #[test]
    fn test_cmd_position_fen_with_moves() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert_eq!(
            cmd_position_fen(fen, &["e7e5"]),
            format!("position fen {fen} moves e7e5")
        );
    }

    // --- go ---

    #[test]
    fn test_cmd_go_default() {
        // GoLimits::default() → no limits → just "go"
        assert_eq!(cmd_go(&GoLimits::default()), "go");
    }

    #[test]
    fn test_cmd_go_infinite() {
        let limits = GoLimits { infinite: true, ..GoLimits::default() };
        assert_eq!(cmd_go(&limits), "go infinite");
    }

    #[test]
    fn test_cmd_go_depth() {
        let limits = GoLimits { depth: Some(12), ..GoLimits::default() };
        assert_eq!(cmd_go(&limits), "go depth 12");
    }

    #[test]
    fn test_cmd_go_movetime() {
        let limits = GoLimits { movetime: Some(1000), ..GoLimits::default() };
        assert_eq!(cmd_go(&limits), "go movetime 1000");
    }

    #[test]
    fn test_cmd_go_time_controls() {
        let limits = GoLimits {
            wtime:     Some(60_000),
            btime:     Some(60_000),
            winc:      Some(1_000),
            binc:      Some(1_000),
            movestogo: Some(40),
            ..GoLimits::default()
        };
        let cmd = cmd_go(&limits);
        assert!(cmd.contains("wtime 60000"),  "wtime manquant");
        assert!(cmd.contains("btime 60000"),  "btime manquant");
        assert!(cmd.contains("winc 1000"),    "winc manquant");
        assert!(cmd.contains("binc 1000"),    "binc manquant");
        assert!(cmd.contains("movestogo 40"), "movestogo manquant");
        assert!(cmd.starts_with("go "),       "doit commencer par 'go '");
    }

    #[test]
    fn test_cmd_go_nodes() {
        let limits = GoLimits { nodes: Some(1_000_000), ..GoLimits::default() };
        assert_eq!(cmd_go(&limits), "go nodes 1000000");
    }

    #[test]
    fn test_cmd_go_mate() {
        let limits = GoLimits { mate: Some(3), ..GoLimits::default() };
        assert_eq!(cmd_go(&limits), "go mate 3");
    }

    #[test]
    fn test_cmd_go_searchmoves() {
        let limits = GoLimits {
            searchmoves: vec!["e2e4".into(), "d2d4".into()],
            depth: Some(10),
            ..GoLimits::default()
        };
        let cmd = cmd_go(&limits);
        assert!(cmd.contains("searchmoves e2e4 d2d4"), "searchmoves manquant");
        assert!(cmd.contains("depth 10"),              "depth manquant");
    }

    #[test]
    fn test_cmd_go_infinite_ignores_other_limits() {
        // When infinite=true, we stop after "go infinite"
        let limits = GoLimits {
            infinite: true,
            depth:    Some(20),
            wtime:    Some(5000),
            ..GoLimits::default()
        };
        assert_eq!(cmd_go(&limits), "go infinite");
    }
}
