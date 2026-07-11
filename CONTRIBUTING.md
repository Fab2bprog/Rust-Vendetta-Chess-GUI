# Contributing to Vendetta Chess GUI

Thanks for your interest in contributing! This document covers everything
you need to get set up, understand the project's conventions, and submit
changes that pass review smoothly.

This project is developed and maintained by a single author so far — there
is no formal team process yet, but contributions are welcome and will be
reviewed as time allows.

## Table of Contents

- [Before you start](#before-you-start)
- [Getting set up](#getting-set-up)
- [Project structure](#project-structure)
- [Coding conventions](#coding-conventions)
- [Quality gates](#quality-gates)
- [Submitting a change](#submitting-a-change)
- [Reporting bugs](#reporting-bugs)
- [License](#license)

## Before you start

For anything beyond a small, obvious fix (typo, small bug, doc improvement),
please open an issue first to discuss the change before writing code. This
avoids wasted effort on a pull request that doesn't fit the project's
direction.

Developer-facing documentation — architecture notes, format specifications,
design decisions — lives in [`docs/`](docs/). Check there first; it may
already answer your question.

## Getting set up

### Requirements
- [Rust](https://rustup.rs) stable (installed via `rustup`).
- One or more UCI chess engines for manual testing (e.g.
  [Stockfish](https://stockfishchess.org)) — not bundled with the
  repository.

### Building

```bash
git clone https://github.com/Fab2bprog/Rust-Vendetta-Chess-GUI.git
cd Rust-Vendetta-Chess-GUI
cargo build
```

Run the application with:

```bash
cargo run --bin vendetta-chess-gui
```

On first launch it creates a `VendettaChess/` data folder next to the
binary — see the main [README](README.md#quick-start) for details.

## Project structure

This is a Rust workspace of 12 crates, each with a single responsibility
(chess logic, UCI communication, engine process management, analysis,
SQLite persistence, SCID database format, tournaments, UI, etc.). See the
[README's architecture section](README.md#architecture) for the full crate
table, and [`docs/`](docs/) for deeper design notes.

The UI (`crates/gui`) is written in [Slint](https://slint.dev), a
declarative `.slint` markup language separate from the Rust code — UI
layout/styling lives in `crates/gui/ui/*.slint`, application logic and
Slint↔Rust wiring live in `crates/gui/src/main.rs`.

## Coding conventions

- **Comments and identifiers are in English.** The codebase used to carry
  French comments from early development; it has since been fully
  translated. New code should follow the same convention.
- **`unsafe` is forbidden** except for a documented, critical justification
  reviewed on a case-by-case basis. If you believe you need `unsafe`, explain
  why in the pull request description before writing it.
- **Formatting** follows the repository's `rustfmt.toml` (100-column width,
  4-space indentation, grouped imports). Run `cargo fmt` before committing —
  don't hand-format.
- **Lint level**: the project builds with `clippy::pedantic` enabled (see
  [Quality gates](#quality-gates) below) — expect to have to address pedantic
  lints, not just the default set.
- Keep crate boundaries clean: the UI never talks to engines or the database
  directly, always through the dedicated service crates (`engine`,
  `analysis`, `db`).

## Quality gates

The same checks that run in CI (`.github/workflows/ci.yml`) should pass
locally before you open a pull request:

```bash
cargo check --all-targets --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings -D clippy::all -D clippy::pedantic -A clippy::module_name_repetitions
cargo test --all-features
```

Compiler and clippy warnings are treated as errors (`RUSTFLAGS="-D
warnings"`) — a pull request with warnings will fail CI.

## Submitting a change

1. Fork the repository and create a branch off `main` for your change.
2. Make your changes, following the conventions above.
3. Run the full [quality gate](#quality-gates) locally.
4. Open a pull request against `main` with a clear description of *what*
   changed and *why*. Reference the related issue if there is one.
5. Be responsive to review feedback — small, focused pull requests are
   easier to review and merge than large ones.

## Reporting bugs

Open a GitHub issue with:
- Your OS and, if relevant, the UCI engine(s) you were using.
- Steps to reproduce.
- What you expected to happen vs. what actually happened.
- Log output from the `logs/` folder if the bug involves a crash or an
  import/analysis failure (debug logging can be enabled in
  Preferences → Misc).

## License

By contributing, you agree that your contributions will be licensed under
the same [GNU GPLv3](LICENSE) license that covers the rest of the project.
