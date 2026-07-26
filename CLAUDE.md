# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

khora is a CLI for web-app QA automation via the Chrome DevTools Protocol. It launches/reconnects to Chrome and drives pages (navigate, find, click, type, screenshot, eval) for CI and agent workflows.

## Commands

```bash
cargo build                                    # debug build -> target/debug/khora
cargo test --workspace --all-targets           # all unit + integration tests
cargo test -p khora-cli --test cli test_help   # single integration test by name
cargo clippy --workspace --all-targets -- -D warnings   # lint (warnings are errors)
cargo fmt --check                              # format check
make setup                                     # install the pre-commit git hook (fmt + clippy)
./tests/e2e/qa.sh                              # end-to-end test; needs Chrome + a debug build
./tests/e2e/key-wedge-soak.sh 25               # soak harness for the intermittent trusted-`key` wedge
```

Diagnostics: `RUST_LOG=khora_cdp=debug khora <cmd>` logs to **stderr** (stdout stays the
machine-readable result, so `khora eval ... | jq` keeps working with logging on). `key_press`
logs the exact serialized `Input.dispatchKeyEvent` params it puts on the wire.

CI (`.github/workflows/ci.yml`) runs check, test, clippy `-D warnings`, and fmt on Linux/macOS/Windows. The pre-commit hook enforces fmt + clippy locally — match it before committing.

## Architecture

Read `docs/architecture.md` first — it is the source of truth for crate layout, session lifecycle, and CDP integration. Key points:

- **3 crates, layered:** `khora-core` (types, config, errors, output — no Chrome deps) → `khora-cdp` (Chrome discovery, `CdpClient` over chromiumoxide, session liveness/reap) → `khora-cli` (clap parser in `main.rs`, dispatches to `CdpClient`).
- **Sessions persist across CLI invocations** as JSON in `~/.khora/sessions/<id>.json` (ID, WebSocket URL, Chrome PID, headless flag, data_dir, plus emulation state that Chrome drops on CDP disconnect and khora re-applies on connect — `media`, `touch`, `viewport`, `keyboard`). Each `launch` gets its own tempdir user-data-dir. Commands reconnect via `load_and_verify()`, which re-checks the PID and returns `SessionDead` if gone.
- **Auto-reap:** every non-`reap` CLI invocation first calls `reap_stale_sessions()` to drop sessions whose PID died. Best-effort — it must never fail the caller.

## Conventions

- **Errors** flow through `KhoraError` (`khora-core/src/error.rs`), which carries exit codes: 1 general, 2 Chrome-not-found, 3 timeout, 4 session-not-found/dead. Add new failure modes here rather than returning ad-hoc strings.
- **Output** is dual-mode via `OutputFormat` (`--format text|json`, or `KHORA_FORMAT`). Any new command must support both; formatting lives in `khora-core/src/output.rs`.
- **Global flags** `--format` and `--timeout` (env `KHORA_FORMAT`, `KHORA_TIMEOUT`, default 5000ms) apply to every subcommand.
- **Adding a command:** add the `Command` variant in `main.rs`, a `CdpClient` method in `khora-cdp/src/client.rs`, a formatter in `output.rs`, a CLI test in `crates/khora-cli/tests/cli.rs`, and a case in `tests/e2e/qa.sh`. Update the README command table and `docs/usage.md`.
- **Element operations** are done by JS evaluation (`page.evaluate()`), not native CDP element calls — see existing methods in `client.rs` for the pattern.

## Releases

Version is set once in `[workspace.package]` in the root `Cargo.toml`. Bump it there; the release workflow (`.github/workflows/release.yml`) and Homebrew tap (`brew install simonspoon/tap/khora`) build from tags.
