//! Parser for UCI messages emitted by the engine.
//!
//! The UCI protocol defines the following messages (engine → GUI):
//!
//! | Message      | Description                                      |
//! |--------------|--------------------------------------------------|
//! | `id`         | Engine name and author                           |
//! | `uciok`      | End of UCI initialization                        |
//! | `readyok`    | Engine ready after `isready`                     |
//! | `bestmove`   | Best move + optional suggested move (ponder)     |
//! | `info`       | Analysis information (score, depth, pv…)         |
//! | `option`     | Declaration of a configurable option             |
//! | `copyprotection` / `registration` | (ignored in the minimal parser) |
//!
//! The [`parse_line`] function turns a raw line into a [`UciMessage`].
//! Unknown lines are returned as [`UciMessage::Unknown`].

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// UCI score as reported by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UciScore {
    /// Score in centipawns (positive = advantage for the side to move).
    Centipawns(i32),
    /// Mate in `n` half-moves (positive = the engine mates, negative = the
    /// engine is mated).
    Mate(i32),
    /// Lower bound.
    Lowerbound(i32),
    /// Upper bound.
    Upperbound(i32),
}

/// Analysis information from an `info` line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UciInfo {
    /// Main search depth.
    pub depth:     Option<u32>,
    /// Maximum selective depth.
    pub seldepth:  Option<u32>,
    /// Search time in ms.
    pub time_ms:   Option<u64>,
    /// Number of nodes analyzed.
    pub nodes:     Option<u64>,
    /// Score.
    pub score:     Option<UciScore>,
    /// Principal variation (list of UCI moves).
    pub pv:        Vec<String>,
    /// `MultiPV` line number.
    pub multipv:   Option<u32>,
    /// Nodes per second.
    pub nps:       Option<u64>,
    /// Hash table fill percentage.
    pub hashfull:  Option<u32>,
    /// Number of endgame tablebase hits.
    pub tbhits:    Option<u64>,
    /// Move currently being analyzed.
    pub currmove:  Option<String>,
    /// Number of the move currently being analyzed.
    pub currmovenumber: Option<u32>,
    /// Free-form text (`string`).
    pub string:    Option<String>,
}

/// Type of a UCI option.
///
/// The UCI protocol defines 5 types:
/// - `spin`   — bounded integer (min/max)
/// - `check`  — boolean (true/false)
/// - `combo`  — choice among a list of values (`var`)
/// - `button` — trigger with no value (e.g. "Clear Hash")
/// - `string` — free-form text string
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UciOptionKind {
    /// Bounded integer: `min`, `max`, and `default` are meaningful.
    Spin,
    /// Boolean: `default` is `"true"` or `"false"`.
    Check,
    /// Choice among `vars`: `default` is one of the values.
    Combo,
    /// Trigger button: no value, no `default`.
    Button,
    /// Free-form text string: `default` may be `<empty>`.
    StringOpt,
    /// Unrecognized type (robustness against proprietary extensions).
    Unknown(String),
}

impl UciOptionKind {
    /// Parses the raw UCI protocol string.
    ///
    /// Named `parse_kind` rather than `from_str` (`clippy::should_implement_trait`,
    /// post-audit fixes from 04/07/2026): avoids confusion with
    /// `std::str::FromStr::from_str`, which this type does not implement
    /// (infallible method, no `Result`/`Err` — not a real `FromStr`).
    #[must_use]
    pub fn parse_kind(s: &str) -> Self {
        match s {
            "spin"   => Self::Spin,
            "check"  => Self::Check,
            "combo"  => Self::Combo,
            "button" => Self::Button,
            "string" => Self::StringOpt,
            other    => Self::Unknown(other.to_owned()),
        }
    }

    /// Returns the UCI protocol string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Spin          => "spin",
            Self::Check         => "check",
            Self::Combo         => "combo",
            Self::Button        => "button",
            Self::StringOpt     => "string",
            Self::Unknown(s)    => s.as_str(),
        }
    }

    /// `true` if the option accepts a value (everything except `Button`).
    #[must_use]
    pub fn has_value(&self) -> bool {
        !matches!(self, Self::Button)
    }
}

/// Option declared by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UciOption {
    pub name:    String,
    /// Type of the option (typed enum).
    pub kind:    UciOptionKind,
    pub default: Option<String>,
    pub min:     Option<i64>,
    pub max:     Option<i64>,
    pub vars:    Vec<String>,
}

/// UCI message emitted by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UciMessage {
    /// `id name <name>`
    IdName(String),
    /// `id author <author>`
    IdAuthor(String),
    /// `uciok`
    UciOk,
    /// `readyok`
    ReadyOk,
    /// `bestmove <move> [ponder <move>]`
    BestMove {
        mv:     String,
        ponder: Option<String>,
    },
    /// `info …`
    Info(UciInfo),
    /// `option name <n> type <t> …`
    Option(UciOption),
    /// Unrecognized line (ignored without error).
    Unknown(String),
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses a raw line from the engine into a [`UciMessage`].
///
/// Never returns an error: unknown lines become
/// [`UciMessage::Unknown`].
#[must_use]
pub fn parse_line(line: &str) -> UciMessage {
    let line = line.trim();

    if line == "uciok"   { return UciMessage::UciOk;   }
    if line == "readyok" { return UciMessage::ReadyOk; }

    if let Some(rest) = line.strip_prefix("id ") {
        return parse_id(rest);
    }
    if let Some(rest) = line.strip_prefix("bestmove ") {
        return parse_bestmove(rest);
    }
    if let Some(rest) = line.strip_prefix("info ") {
        return UciMessage::Info(parse_info(rest));
    }
    if let Some(rest) = line.strip_prefix("option ") {
        return UciMessage::Option(parse_option(rest));
    }

    UciMessage::Unknown(line.to_owned())
}

// ---------------------------------------------------------------------------
// Internal parsers
// ---------------------------------------------------------------------------

fn parse_id(rest: &str) -> UciMessage {
    if let Some(name) = rest.strip_prefix("name ") {
        return UciMessage::IdName(name.trim().to_owned());
    }
    if let Some(author) = rest.strip_prefix("author ") {
        return UciMessage::IdAuthor(author.trim().to_owned());
    }
    UciMessage::Unknown(format!("id {rest}"))
}

fn parse_bestmove(rest: &str) -> UciMessage {
    let mut tokens = rest.split_whitespace();
    let mv = match tokens.next() {
        Some(m) => m.to_owned(),
        None    => return UciMessage::Unknown(format!("bestmove {rest}")),
    };

    // optional ponder
    let ponder = if tokens.next() == Some("ponder") {
        tokens.next().map(str::to_owned)
    } else {
        None
    };

    UciMessage::BestMove { mv, ponder }
}

/// Parses the tokens of an `info` line into a [`UciInfo`].
fn parse_info(rest: &str) -> UciInfo {
    let mut info  = UciInfo::default();
    let mut tokens = rest.split_whitespace().peekable();

    while let Some(token) = tokens.next() {
        match token {
            "depth"    => info.depth    = tokens.next().and_then(|v| v.parse().ok()),
            "seldepth" => info.seldepth = tokens.next().and_then(|v| v.parse().ok()),
            "time"     => info.time_ms  = tokens.next().and_then(|v| v.parse().ok()),
            "nodes"    => info.nodes    = tokens.next().and_then(|v| v.parse().ok()),
            "nps"      => info.nps      = tokens.next().and_then(|v| v.parse().ok()),
            "multipv"  => info.multipv  = tokens.next().and_then(|v| v.parse().ok()),
            "hashfull" => info.hashfull = tokens.next().and_then(|v| v.parse().ok()),
            "tbhits"   => info.tbhits   = tokens.next().and_then(|v| v.parse().ok()),
            "currmovenumber" => info.currmovenumber = tokens.next().and_then(|v| v.parse().ok()),
            "currmove" => info.currmove = tokens.next().map(str::to_owned),
            "score"    => info.score    = parse_score(&mut tokens),
            "pv"       => {
                // pv consumes all remaining tokens
                info.pv = tokens.by_ref().map(str::to_owned).collect();
                break;
            }
            "string"   => {
                // string consumes all remaining tokens
                info.string = Some(tokens.by_ref().collect::<Vec<_>>().join(" "));
                break;
            }
            _ => {} // unknown token, ignored
        }
    }

    info
}

fn parse_score(
    tokens: &mut std::iter::Peekable<std::str::SplitWhitespace<'_>>,
) -> Option<UciScore> {
    match tokens.next()? {
        "cp" => {
            let v: i32 = tokens.next()?.parse().ok()?;
            // Check for lowerbound / upperbound
            match tokens.peek().copied() {
                Some("lowerbound") => { tokens.next(); Some(UciScore::Lowerbound(v)) }
                Some("upperbound") => { tokens.next(); Some(UciScore::Upperbound(v)) }
                _ => Some(UciScore::Centipawns(v)),
            }
        }
        "mate" => {
            let v: i32 = tokens.next()?.parse().ok()?;
            Some(UciScore::Mate(v))
        }
        _ => None,
    }
}

fn parse_option(rest: &str) -> UciOption {
    let mut opt = UciOption {
        name:    String::new(),
        kind:    UciOptionKind::Unknown(String::new()),
        default: None,
        min:     None,
        max:     None,
        vars:    Vec::new(),
    };

    // We tokenize manually to handle multi-word fields (name, var…).
    // Recognized keys: name, type, default, min, max, var
    let keywords = ["name", "type", "default", "min", "max", "var"];
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;

    while i < tokens.len() {
        if keywords.contains(&tokens[i]) {
            let key = tokens[i];
            i += 1;
            // Collects words until the next keyword
            let mut value_parts: Vec<&str> = Vec::new();
            while i < tokens.len() && !keywords.contains(&tokens[i]) {
                value_parts.push(tokens[i]);
                i += 1;
            }
            let value = value_parts.join(" ");
            match key {
                "name"    => opt.name    = value,
                "type"    => opt.kind    = UciOptionKind::parse_kind(&value),
                "default" => opt.default = Some(value),
                "min"     => opt.min     = value.parse().ok(),
                "max"     => opt.max     = value.parse().ok(),
                "var"     => opt.vars.push(value),
                _         => {}
            }
        } else {
            i += 1;
        }
    }

    opt
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Simple tokens ---

    #[test]
    fn test_parse_uciok() {
        assert_eq!(parse_line("uciok"), UciMessage::UciOk);
    }

    #[test]
    fn test_parse_readyok() {
        assert_eq!(parse_line("readyok"), UciMessage::ReadyOk);
    }

    #[test]
    fn test_parse_uciok_with_whitespace() {
        assert_eq!(parse_line("  uciok  "), UciMessage::UciOk);
    }

    // --- id ---

    #[test]
    fn test_parse_id_name() {
        assert_eq!(
            parse_line("id name Stockfish 16"),
            UciMessage::IdName("Stockfish 16".into())
        );
    }

    #[test]
    fn test_parse_id_author() {
        assert_eq!(
            parse_line("id author T. Romstad, M. Costalba"),
            UciMessage::IdAuthor("T. Romstad, M. Costalba".into())
        );
    }

    // --- bestmove ---

    #[test]
    fn test_parse_bestmove_simple() {
        assert_eq!(
            parse_line("bestmove e2e4"),
            UciMessage::BestMove { mv: "e2e4".into(), ponder: None }
        );
    }

    #[test]
    fn test_parse_bestmove_with_ponder() {
        assert_eq!(
            parse_line("bestmove e2e4 ponder e7e5"),
            UciMessage::BestMove {
                mv:     "e2e4".into(),
                ponder: Some("e7e5".into()),
            }
        );
    }

    #[test]
    fn test_parse_bestmove_promotion() {
        assert_eq!(
            parse_line("bestmove e7e8q"),
            UciMessage::BestMove { mv: "e7e8q".into(), ponder: None }
        );
    }

    // --- info ---

    #[test]
    fn test_parse_info_depth_nodes() {
        let msg = parse_line("info depth 12 nodes 123456 time 500");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.depth,   Some(12));
            assert_eq!(info.nodes,   Some(123_456));
            assert_eq!(info.time_ms, Some(500));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_cp() {
        let msg = parse_line("info depth 8 score cp 42");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Centipawns(42)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_negative_cp() {
        let msg = parse_line("info depth 8 score cp -150");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Centipawns(-150)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_mate() {
        let msg = parse_line("info depth 5 score mate 3");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Mate(3)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_mate_negative() {
        let msg = parse_line("info depth 5 score mate -2");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Mate(-2)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_lowerbound() {
        let msg = parse_line("info depth 10 score cp 30 lowerbound");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Lowerbound(30)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_score_upperbound() {
        let msg = parse_line("info depth 10 score cp -20 upperbound");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.score, Some(UciScore::Upperbound(-20)));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_pv() {
        let msg = parse_line("info depth 10 score cp 30 pv e2e4 e7e5 g1f3");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.pv, ["e2e4", "e7e5", "g1f3"]);
            assert_eq!(info.depth, Some(10));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_multipv() {
        let msg = parse_line("info multipv 2 depth 8 score cp -10 pv d7d5");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.multipv, Some(2));
            assert_eq!(info.pv, ["d7d5"]);
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_string() {
        let msg = parse_line("info string NNUE evaluation using nn-abc123.nnue");
        if let UciMessage::Info(info) = msg {
            assert_eq!(
                info.string.as_deref(),
                Some("NNUE evaluation using nn-abc123.nnue")
            );
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_currmove() {
        let msg = parse_line("info currmove e2e4 currmovenumber 1");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.currmove.as_deref(), Some("e2e4"));
            assert_eq!(info.currmovenumber, Some(1));
        } else {
            panic!("Attendu Info");
        }
    }

    #[test]
    fn test_parse_info_nps_hashfull_tbhits() {
        let msg = parse_line("info nps 1500000 hashfull 450 tbhits 12");
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.nps,      Some(1_500_000));
            assert_eq!(info.hashfull, Some(450));
            assert_eq!(info.tbhits,   Some(12));
        } else {
            panic!("Attendu Info");
        }
    }

    // --- option ---

    #[test]
    fn test_parse_option_spin() {
        let msg = parse_line("option name Hash type spin default 16 min 1 max 33554432");
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name,               "Hash");
            assert_eq!(opt.kind,               UciOptionKind::Spin);
            assert_eq!(opt.default.as_deref(), Some("16"));
            assert_eq!(opt.min,                Some(1));
            assert_eq!(opt.max,                Some(33_554_432));
            assert!(opt.kind.has_value());
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_parse_option_check() {
        let msg = parse_line("option name Ponder type check default false");
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name,               "Ponder");
            assert_eq!(opt.kind,               UciOptionKind::Check);
            assert_eq!(opt.default.as_deref(), Some("false"));
            assert!(opt.kind.has_value());
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_parse_option_combo() {
        let msg = parse_line(
            "option name UCI_AnalyseMode type combo default false var false var true",
        );
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name, "UCI_AnalyseMode");
            assert_eq!(opt.kind, UciOptionKind::Combo);
            assert_eq!(opt.vars, ["false", "true"]);
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_parse_option_button() {
        let msg = parse_line("option name Clear Hash type button");
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name, "Clear Hash");
            assert_eq!(opt.kind, UciOptionKind::Button);
            assert!(opt.default.is_none());
            assert!(!opt.kind.has_value());
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_parse_option_string() {
        let msg = parse_line("option name NalimovPath type string default <empty>");
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name,               "NalimovPath");
            assert_eq!(opt.kind,               UciOptionKind::StringOpt);
            assert_eq!(opt.default.as_deref(), Some("<empty>"));
            assert!(opt.kind.has_value());
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_parse_option_unknown_kind() {
        let msg = parse_line("option name FutureOption type futuretype default 42");
        if let UciMessage::Option(opt) = msg {
            assert_eq!(opt.name, "FutureOption");
            assert!(matches!(opt.kind, UciOptionKind::Unknown(_)));
            assert_eq!(opt.kind.as_str(), "futuretype");
        } else {
            panic!("Attendu Option");
        }
    }

    #[test]
    fn test_option_kind_as_str() {
        assert_eq!(UciOptionKind::Spin.as_str(),      "spin");
        assert_eq!(UciOptionKind::Check.as_str(),     "check");
        assert_eq!(UciOptionKind::Combo.as_str(),     "combo");
        assert_eq!(UciOptionKind::Button.as_str(),    "button");
        assert_eq!(UciOptionKind::StringOpt.as_str(), "string");
    }

    #[test]
    fn test_option_kind_has_value() {
        assert!(UciOptionKind::Spin.has_value());
        assert!(UciOptionKind::Check.has_value());
        assert!(UciOptionKind::Combo.has_value());
        assert!(UciOptionKind::StringOpt.has_value());
        assert!(!UciOptionKind::Button.has_value());
    }

    // --- unknown ---

    #[test]
    fn test_parse_unknown() {
        let msg = parse_line("quelque chose d inconnu");
        assert!(matches!(msg, UciMessage::Unknown(_)));
    }

    #[test]
    fn test_parse_empty_line() {
        let msg = parse_line("");
        assert!(matches!(msg, UciMessage::Unknown(_)));
    }

    // --- Real Stockfish line ---

    #[test]
    fn test_parse_stockfish_info_line() {
        let line = "info depth 20 seldepth 28 multipv 1 score cp 28 nodes 1234567 \
                    nps 2345678 hashfull 123 tbhits 0 time 526 pv e2e4 e7e5 g1f3";
        let msg = parse_line(line);
        if let UciMessage::Info(info) = msg {
            assert_eq!(info.depth,    Some(20));
            assert_eq!(info.seldepth, Some(28));
            assert_eq!(info.multipv,  Some(1));
            assert_eq!(info.score,    Some(UciScore::Centipawns(28)));
            assert_eq!(info.nodes,    Some(1_234_567));
            assert_eq!(info.nps,      Some(2_345_678));
            assert_eq!(info.hashfull, Some(123));
            assert_eq!(info.tbhits,   Some(0));
            assert_eq!(info.time_ms,  Some(526));
            assert_eq!(info.pv,       ["e2e4", "e7e5", "g1f3"]);
        } else {
            panic!("Attendu Info");
        }
    }
}
