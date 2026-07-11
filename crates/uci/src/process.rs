//! Management of the UCI engine process.
//!
//! [`EngineProcess`] launches an external engine (Stockfish, Komodo, etc.) as
//! a child process. A dedicated thread continuously reads stdout and pushes
//! each line into an `mpsc::channel`, which allows real timeouts via
//! `recv_timeout` without blocking the calling thread.

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

/// Maximum number of stderr lines kept for diagnostics (see
/// [`EngineProcess::last_stderr_lines`]). Bounded to avoid unlimited memory
/// growth if an engine loops while writing to stderr.
const STDERR_HISTORY_LEN: usize = 20;

/// Hard ceiling, in bytes, on a single line read from the engine's
/// stdout by [`read_bounded_line`] (robustness audit 11/07/2026, finding
/// 3.5) — far beyond any real UCI line (even a `go` response with a
/// very deep `MultiPV` principal variation stays a few KB at most): if
/// the process pointed to isn't actually a UCI engine (any executable,
/// selected by mistake) and writes a continuous stream of bytes with no
/// `\n`, this stops the accumulation instead of growing memory without
/// bound.
const MAX_LINE_BYTES: usize = 1_000_000;

/// Reads one line from `reader`, bounded by [`MAX_LINE_BYTES`] — a
/// hand-rolled, bounded, UTF-8-tolerant replacement for
/// `BufRead::lines()`'s `next()` (robustness audit 11/07/2026, findings
/// 3.5 and 3.6):
/// - 3.5: `BufRead::lines()` has no length limit on a single line (see
///   [`MAX_LINE_BYTES`]'s doc).
/// - 3.6: `BufRead::lines()` also validates UTF-8 internally and
///   returns `Err` on the very first invalid byte, which the caller
///   used to treat as a fatal, permanent disconnection — even if the
///   underlying process is still alive and functioning otherwise (e.g.
///   one stray non-UTF-8 byte in an otherwise harmless `info string`
///   line, such as engine-specific loading diagnostics). Reading raw
///   bytes and converting with [`String::from_utf8_lossy`] (replacement
///   character instead of a hard error) avoids this.
///
/// Returns `Ok(None)` at a clean EOF with nothing left to read,
/// `Ok(Some(line))` for a line (trailing `\r`/`\n` stripped, matching
/// `BufRead::lines()`'s own convention), or `Err(())` if a real I/O
/// error occurs or a single line exceeds [`MAX_LINE_BYTES`] before any
/// `\n` is found — treated the same as a disconnection by the caller:
/// abandoning the connection is preferable to risking unbounded memory
/// growth on a process that isn't behaving like a UCI engine at all.
fn read_bounded_line<R: BufRead>(reader: &mut R) -> Result<Option<String>, ()> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| ())?;
        if available.is_empty() {
            // Clean EOF: whatever was accumulated (if anything, an
            // unterminated final line with no trailing `\n`) is the last
            // line; nothing at all means the stream is simply done.
            return if buf.is_empty() {
                Ok(None)
            } else {
                Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
            };
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..=pos]);
            reader.consume(pos + 1);
            while matches!(buf.last(), Some(b'\n' | b'\r')) {
                buf.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let consumed = available.len();
        if buf.len() + consumed > MAX_LINE_BYTES {
            return Err(());
        }
        buf.extend_from_slice(available);
        reader.consume(consumed);
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Error from the UCI process manager.
#[derive(Debug)]
pub enum ProcessError {
    /// Unable to launch the process (binary not found, permissions…).
    SpawnFailed(std::io::Error),
    /// Write to stdin failed (process dead?).
    WriteFailed(std::io::Error),
    /// Missing stdin/stdout pipe (should not happen after `spawn`).
    PipeMissing,
    /// Timeout exceeded while waiting for a response.
    Timeout,
    /// The engine closed its stdout (terminated unexpectedly).
    Disconnected,
    /// The command contains a `\n`/`\r` control character (robustness
    /// audit 11/07/2026, finding 3.7) — rejected before being written to
    /// stdin rather than silently stripped, since its presence always
    /// signals malformed/corrupted input (e.g. a hand-edited engine
    /// config file with a corrupted UCI option value) worth surfacing
    /// rather than masking. Without this check, such a value would
    /// otherwise inject an arbitrary extra UCI command into the engine's
    /// stdin stream — e.g. a rogue option value of `"1\nquit"` would
    /// silently make the engine quit mid-game.
    InvalidCommand(String),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e)  => write!(f, "Impossible de lancer le moteur : {e}"),
            Self::WriteFailed(e)  => write!(f, "Erreur d'écriture stdin : {e}"),
            Self::PipeMissing     => write!(f, "Pipe stdin/stdout manquant"),
            Self::Timeout         => write!(f, "Timeout dépassé"),
            Self::Disconnected    => write!(f, "Moteur déconnecté"),
            Self::InvalidCommand(cmd) => write!(
                f,
                "Commande UCI invalide (caractère de contrôle détecté) : {cmd:?}"
            ),
        }
    }
}

impl std::error::Error for ProcessError {}

// ---------------------------------------------------------------------------
// EngineProcess
// ---------------------------------------------------------------------------

/// Active UCI process with a dedicated reader thread.
pub struct EngineProcess {
    child:    Child,
    stdin:    ChildStdin,
    /// Receiver for the lines read by the reader thread.
    receiver: Receiver<String>,
    /// Latest stderr lines from the engine (diagnostics in case of a crash).
    /// Shared with the dedicated stderr reader thread.
    stderr_history: Arc<Mutex<VecDeque<String>>>,
}

impl std::fmt::Debug for EngineProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `stderr_history` deliberately omitted (clippy::missing_fields_in_debug,
        // post-audit fixes from 04/07/2026): locking it from `fmt` would risk
        // a deadlock if called while another thread already holds it —
        // `finish_non_exhaustive` honestly signals that this `Debug` is
        // partial rather than pretending otherwise.
        f.debug_struct("EngineProcess")
            .field("child", &self.child)
            .field("stdin", &self.stdin)
            .field("receiver", &"Receiver<String>")
            .finish_non_exhaustive()
    }
}

/// Defensively reapplies the Unix executable bit on `path` before every
/// launch (PHASE 24, USB portability).
///
/// exFAT/FAT32 — common formats for a USB drive readable by both Windows and
/// macOS — do not preserve Unix permission bits: an engine copied into
/// `moteurs/` on one computer can lose its `+x` bit once the USB drive is
/// plugged into another. Single launch point for all of the application's
/// engines (advice, game, tournament, analysis): `UciEngine::connect`/
/// `connect_with_timeout` all call `EngineProcess::spawn`, so this fix
/// applies everywhere without duplicating the `chmod` call at every call
/// site.
///
/// Best-effort and silent: if `path` is not a direct file path (e.g.
/// `"cat"` resolved via `$PATH`) or if reading/writing permissions fails,
/// `Command::spawn` is left to fail normally with its own error message —
/// no less robust than before this fix.
#[cfg(unix)]
fn ensure_executable_bit(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut perms = metadata.permissions();
        let mode = perms.mode();
        if mode & 0o111 != 0o111 {
            perms.set_mode(mode | 0o111);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

impl EngineProcess {
    /// Launches the engine located at `path`.
    ///
    /// A reader thread is started immediately to consume stdout, along with
    /// a second thread dedicated to stderr (diagnostics only — see
    /// [`Self::last_stderr_lines`]).
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::SpawnFailed`] if the binary cannot be found.
    pub fn spawn(path: &str) -> Result<Self, ProcessError> {
        #[cfg(unix)]
        ensure_executable_bit(path);

        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // PHASE 75 — Windows opens a new console window by default for any
        // "console"-type child process (the case for practically every UCI
        // engine), even though stdin/stdout/stderr are already redirected to
        // pipes here — this window was never needed for the engine to
        // function. `CREATE_NO_WINDOW` suppresses its creation without
        // changing anything else (the stdio pipes remain fully functional).
        // Symptom reported by the user: a black window visible every time an
        // engine launched, and closing it by mistake killed the engine (the
        // close button sends a signal to the console AND to the process
        // attached to it).
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            /// `CREATE_NO_WINDOW` (Win32 constant, absent from `std` — see
            /// the official Microsoft `CreateProcess` documentation).
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(ProcessError::SpawnFailed)?;

        let stdin  = child.stdin.take().ok_or(ProcessError::PipeMissing)?;
        let stdout = child.stdout.take().ok_or(ProcessError::PipeMissing)?;
        let stderr = child.stderr.take().ok_or(ProcessError::PipeMissing)?;

        // Reader thread: reads stdout line by line and sends into the channel.
        let (tx, rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            // `while let` rather than `loop` + `match` (clippy::while_let_loop):
            // clean EOF (`Ok(None)`), a real I/O error, or a single line over
            // MAX_LINE_BYTES (both `Err(())`) all fall through to the same
            // place — the condition simply stops matching and the loop ends,
            // with no separate arm needed to spell that out.
            while let Ok(Some(line)) = read_bounded_line(&mut reader) {
                if tx.send(line).is_err() {
                    break; // Receiver dropped → stop
                }
            }
        });

        // Stderr reader thread: keeps only the last N lines for
        // diagnostics (e.g. crash message shown in GUI logs).
        // Previously `Stdio::null()` — any error information printed by a
        // third-party engine before dying was silently lost.
        let stderr_history = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_HISTORY_LEN)));
        {
            let history = stderr_history.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut hist) = history.lock() {
                        if hist.len() >= STDERR_HISTORY_LEN {
                            hist.pop_front();
                        }
                        hist.push_back(line);
                    }
                }
            });
        }

        Ok(Self { child, stdin, receiver: rx, stderr_history })
    }

    /// Latest stderr lines emitted by the engine (at most
    /// [`STDERR_HISTORY_LEN`]), useful for diagnosing a crash.
    /// Returns an empty vector if the engine wrote nothing to stderr.
    #[must_use]
    pub fn last_stderr_lines(&self) -> Vec<String> {
        self.stderr_history
            .lock()
            .map(|hist| hist.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Sends a command to the engine (automatically appends `\n`).
    ///
    /// # Errors
    ///
    /// - [`ProcessError::InvalidCommand`] if `cmd` contains a `\n`/`\r`
    ///   (robustness audit 11/07/2026, finding 3.7) — see its doc for why.
    /// - [`ProcessError::WriteFailed`] if the write to stdin fails.
    pub fn send_command(&mut self, cmd: &str) -> Result<(), ProcessError> {
        if cmd.contains(['\n', '\r']) {
            return Err(ProcessError::InvalidCommand(cmd.to_owned()));
        }
        self.stdin
            .write_all(cmd.as_bytes())
            .map_err(ProcessError::WriteFailed)?;
        self.stdin
            .write_all(b"\n")
            .map_err(ProcessError::WriteFailed)?;
        self.stdin.flush().map_err(ProcessError::WriteFailed)
    }

    /// Reads the next available line without waiting (non-blocking).
    ///
    /// Returns `Ok(None)` if nothing is available right now.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::Disconnected`] if the engine closed stdout.
    pub fn try_read_line(&mut self) -> Result<Option<String>, ProcessError> {
        match self.receiver.try_recv() {
            Ok(line)                      => Ok(Some(line)),
            Err(TryRecvError::Empty)      => Ok(None),
            Err(TryRecvError::Disconnected) => Err(ProcessError::Disconnected),
        }
    }

    /// Reads the next line, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// - [`ProcessError::Timeout`] if no line arrives within the delay.
    /// - [`ProcessError::Disconnected`] if the engine closed stdout.
    pub fn read_line_timeout(&mut self, timeout: Duration) -> Result<String, ProcessError> {
        self.receiver
            .recv_timeout(timeout)
            .map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout      => ProcessError::Timeout,
                mpsc::RecvTimeoutError::Disconnected => ProcessError::Disconnected,
            })
    }

    /// Reads lines until `predicate` is satisfied or `timeout` is exceeded.
    /// Returns all lines read.
    ///
    /// The timeout applies to **each** individual line, not to the total.
    ///
    /// # Errors
    ///
    /// - [`ProcessError::Timeout`] if a line does not arrive within `timeout`.
    /// - [`ProcessError::Disconnected`] if the engine closes stdout.
    pub fn read_lines_until<F>(
        &mut self,
        predicate: F,
        timeout: Duration,
    ) -> Result<Vec<String>, ProcessError>
    where
        F: Fn(&str) -> bool,
    {
        let mut lines = Vec::new();
        loop {
            let line = self.read_line_timeout(timeout)?;
            let done = predicate(&line);
            lines.push(line);
            if done {
                return Ok(lines);
            }
        }
    }

    /// Sends `quit` and waits for the child process to terminate.
    ///
    /// The wait is bounded by [`QUIT_WAIT_TIMEOUT`]: if the engine does not
    /// terminate on its own within this delay (stuck, ignores `quit`), the
    /// process is force-killed to avoid any indefinite blocking of the
    /// caller (notably a `Drop` triggered from the UI thread).
    pub fn quit(mut self) {
        let _ = self.send_command("quit");
        wait_with_timeout(&mut self.child, QUIT_WAIT_TIMEOUT);
    }

    /// Kills the process immediately (use only as a last resort).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        wait_with_timeout(&mut self.child, QUIT_WAIT_TIMEOUT);
    }

    /// Returns `true` if the process is still running.
    #[must_use]
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Safety net: if `quit()`/`kill()` were never explicitly called (e.g.
/// `panic!` between `spawn()` and the explicit call), this ensures no engine
/// process is left orphaned/zombie in the background.
impl Drop for EngineProcess {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            wait_with_timeout(&mut self.child, QUIT_WAIT_TIMEOUT);
        }
    }
}

/// Maximum delay granted to an engine process to terminate after
/// `quit`/`kill` before being considered stuck.
const QUIT_WAIT_TIMEOUT: Duration = Duration::from_secs(3);

/// Polling interval used by [`wait_with_timeout`].
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Waits for `child` to terminate, bounding the wait to `timeout`
/// (non-blocking poll via `try_wait`, since `std::process::Child` does not
/// expose a native `wait_timeout`). If the delay is exceeded, force-kills
/// the process then waits for its final termination (which is normally
/// immediate after a `kill()` succeeds at the OS level).
fn wait_with_timeout(child: &mut Child, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                thread::sleep(WAIT_POLL_INTERVAL);
            }
            Err(_) => return,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── read_bounded_line (robustness audit 11/07/2026, findings 3.5/3.6) ──
    // Exercised directly against an in-memory `std::io::Cursor` (a
    // `BufRead`) rather than a real process — portable (no `#[cfg(unix)]`
    // needed) and deterministic.

    #[test]
    fn test_read_bounded_line_basic_lines() {
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(b"hello\nworld\n".to_vec()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some("hello".to_owned()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some("world".to_owned()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
    }

    #[test]
    fn test_read_bounded_line_strips_crlf() {
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(b"uciok\r\n".to_vec()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some("uciok".to_owned()));
    }

    #[test]
    fn test_read_bounded_line_no_trailing_newline_at_eof() {
        // A final, unterminated line (process died mid-write) is still
        // returned once, then `None` on the next call — not lost.
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(b"bestmove e2e4".to_vec()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some("bestmove e2e4".to_owned()));
        assert_eq!(read_bounded_line(&mut reader).unwrap(), None);
    }

    #[test]
    fn test_read_bounded_line_invalid_utf8_is_lossy_not_fatal() {
        // Finding 3.6: a stray non-UTF-8 byte must not kill the reader —
        // it is replaced (U+FFFD), and the NEXT line is still readable
        // normally (the connection is not treated as dead).
        let mut data = b"info string \xFF broken\n".to_vec();
        data.extend_from_slice(b"bestmove e2e4\n");
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(data));
        let first = read_bounded_line(&mut reader).unwrap().unwrap();
        assert!(first.contains('\u{FFFD}'), "ligne obtenue : {first:?}");
        assert_eq!(read_bounded_line(&mut reader).unwrap(), Some("bestmove e2e4".to_owned()));
    }

    #[test]
    fn test_read_bounded_line_rejects_line_over_max_bytes() {
        // Finding 3.5: a line with no `\n` at all, longer than
        // `MAX_LINE_BYTES`, must not be buffered indefinitely — the
        // connection is abandoned (`Err`) instead of growing memory
        // without bound.
        let data = vec![b'a'; MAX_LINE_BYTES + 1];
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(data));
        assert!(read_bounded_line(&mut reader).is_err());
    }

    #[test]
    fn test_read_bounded_line_accepts_line_at_exactly_max_bytes() {
        // Boundary check: exactly `MAX_LINE_BYTES` followed by `\n` must
        // still succeed (off-by-one safety on the cap itself).
        let mut data = vec![b'a'; MAX_LINE_BYTES];
        data.push(b'\n');
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(data));
        let line = read_bounded_line(&mut reader).unwrap().unwrap();
        assert_eq!(line.len(), MAX_LINE_BYTES);
    }

    // We use `cat` (Unix) as a fake engine: it returns exactly what is sent
    // to it, which is enough to test send/read.

    #[cfg(unix)]
    fn spawn_cat() -> EngineProcess {
        EngineProcess::spawn("cat").expect("cat introuvable")
    }

    #[cfg(unix)]
    #[test]
    fn test_spawn_and_is_running() {
        let mut p = spawn_cat();
        assert!(p.is_running());
        p.kill();
        assert!(!p.is_running());
    }

    // ── send_command control-character rejection (robustness audit
    //    11/07/2026, finding 3.7) ─────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn test_send_command_rejects_embedded_newline() {
        // A `\n` inside the command would otherwise inject an arbitrary
        // extra UCI command into the engine's stdin stream.
        let mut p = spawn_cat();
        let result = p.send_command("setoption name Threads value 1\nquit");
        assert!(matches!(result, Err(ProcessError::InvalidCommand(_))), "résultat obtenu : {result:?}");
        // The connection itself is untouched: a normal command right
        // after still goes through.
        p.send_command("still alive").unwrap();
        let line = p.read_line_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(line, "still alive");
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_send_command_rejects_embedded_carriage_return() {
        let mut p = spawn_cat();
        let result = p.send_command("go movetime 100\rstop");
        assert!(matches!(result, Err(ProcessError::InvalidCommand(_))), "résultat obtenu : {result:?}");
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_send_and_read_line() {
        let mut p = spawn_cat();
        p.send_command("hello").unwrap();
        let line = p.read_line_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(line, "hello");
        p.kill();
    }

    // ── Defensive executable bit (PHASE 24, USB portability) ───────────────

    /// Locates a real `cat` binary on disk (not just resolved via
    /// `$PATH`), needed to test `ensure_executable_bit`, which requires a
    /// direct file path. Passes silently if absent (unusual CI), same
    /// principle as `crates/engine/src/config.rs`.
    #[cfg(unix)]
    fn real_cat_path() -> Option<std::path::PathBuf> {
        ["/bin/cat", "/usr/bin/cat"]
            .into_iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())
    }

    #[cfg(unix)]
    #[test]
    fn test_spawn_reapplies_missing_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let Some(cat) = real_cat_path() else { return };

        let tmp = std::env::temp_dir()
            .join(format!("vendetta_test_no_exec_bit_{}", std::process::id()));
        std::fs::copy(&cat, &tmp).expect("copie de cat");
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))
            .expect("retrait du bit +x");

        let path_str = tmp.to_string_lossy().into_owned();
        let mut p = EngineProcess::spawn(&path_str)
            .expect("spawn doit réussir malgré le bit +x manquant au départ");
        assert!(p.is_running());
        p.kill();

        let _ = std::fs::remove_file(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_executable_bit_is_noop_when_already_executable() {
        use std::os::unix::fs::PermissionsExt;

        let Some(cat) = real_cat_path() else { return };
        let mode_before = std::fs::metadata(&cat).unwrap().permissions().mode();

        ensure_executable_bit(&cat.to_string_lossy());

        let mode_after = std::fs::metadata(&cat).unwrap().permissions().mode();
        assert_eq!(mode_before, mode_after);
    }

    #[cfg(unix)]
    #[test]
    fn test_ensure_executable_bit_silently_ignores_unresolvable_path() {
        // "cat" alone (resolved via $PATH, not a direct file path):
        // std::fs::metadata fails, the function must not panic.
        ensure_executable_bit("cat");
        ensure_executable_bit("/definitely/does/not/exist/engine");
    }

    #[cfg(unix)]
    #[test]
    fn test_send_multiple_commands() {
        let mut p = spawn_cat();
        p.send_command("uci").unwrap();
        p.send_command("isready").unwrap();
        let l1 = p.read_line_timeout(Duration::from_secs(2)).unwrap();
        let l2 = p.read_line_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(l1, "uci");
        assert_eq!(l2, "isready");
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_read_lines_until() {
        let mut p = spawn_cat();
        p.send_command("line1").unwrap();
        p.send_command("line2").unwrap();
        p.send_command("STOP").unwrap();

        let lines = p
            .read_lines_until(|l| l == "STOP", Duration::from_secs(2))
            .unwrap();

        assert_eq!(lines, ["line1", "line2", "STOP"]);
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_timeout_on_silence() {
        let mut p = spawn_cat();
        // Cat returns nothing → real timeout thanks to the channel
        let result = p.read_line_timeout(Duration::from_millis(100));
        assert!(matches!(result.unwrap_err(), ProcessError::Timeout));
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_try_read_line_empty() {
        let mut p = spawn_cat();
        // Nothing sent → immediate None
        let result = p.try_read_line().unwrap();
        assert!(result.is_none());
        p.kill();
    }

    #[test]
    fn test_spawn_invalid_path() {
        let result = EngineProcess::spawn("/bin/moteur_qui_nexiste_pas");
        assert!(matches!(result.unwrap_err(), ProcessError::SpawnFailed(_)));
    }

    #[cfg(unix)]
    fn create_stderr_script() -> String {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let script = "#!/bin/sh\necho 'erreur fatale de test' >&2\ncat\n";
        let path = std::env::temp_dir().join("vendetta_stderr_test_engine.sh");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn test_last_stderr_lines_captures_output() {
        let path = create_stderr_script();
        let mut p = EngineProcess::spawn(&path).expect("script introuvable");

        // The stderr reader thread is asynchronous: we wait briefly for it
        // to have time to consume the line (poll bounded to 3 s — widened
        // from 1 s, robustness audit 11/07/2026 follow-up: observed
        // flaky under a full `cargo test` run, where dozens of other
        // tests in this same file/crate concurrently spawn their own
        // `sh`/`cat` child processes, occasionally delaying this
        // particular thread's OS scheduling past a 1 s budget even
        // though the script itself runs near-instantly in isolation).
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut lines = Vec::new();
        while std::time::Instant::now() < deadline {
            lines = p.last_stderr_lines();
            if !lines.is_empty() { break; }
            thread::sleep(Duration::from_millis(20));
        }

        assert_eq!(lines, vec!["erreur fatale de test".to_owned()]);
        p.kill();
    }

    #[cfg(unix)]
    #[test]
    fn test_last_stderr_lines_empty_when_silent() {
        let mut p = spawn_cat();
        // cat writes nothing to stderr
        assert!(p.last_stderr_lines().is_empty());
        p.kill();
    }
}
