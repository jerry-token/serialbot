# SerialBot — working rules

SerialBot (`serialbot` for binaries, crates and packages) is a lightweight, cross-platform serial
debugging assistant whose AI agent ("Bo") and MCP server can drive serial
ports. Plan of record: the "AI 串口助手 · 开工前计划" doc.

## Quality bar (non-negotiable)

`cargo xtask check` runs every check below, in CI's order, and prints a
summary; CI runs the same checks. Run `cargo xtask hooks` once per clone:
the pre-commit hook runs `cargo xtask check --fast` (everything but the slow
checks marked *slow*), the commit-msg hook checks the message.

| Rule | Enforced by |
|---|---|
| Every change ships with tests written in the same change (docs/ci/chore/style/build/revert commits excepted). Bug fixes start with a failing test. | `check:tests-with-change` |
| Code is formatted. | `check:fmt` |
| No warnings: clippy with `-D warnings`. This also enforces no `unwrap()`/`expect()` outside tests, no `print!` outside the CLI's output module, doc comments on public items (`missing_docs`) and no `unsafe` (forbidden). | `check:clippy` |
| Those lints stay switched on, and every crate inherits them. | `check:lint-config`, `check:crate-lints` |
| All tests pass. | `check:test` |
| Docs build without warnings (broken links fail). *slow* | `check:docs` |
| Rust line coverage stays at or above 85% (`cargo llvm-cov`). *slow* | `check:coverage` |
| Frontend: typecheck, line coverage at or above 85%, build. *slow* | `check:frontend` |
| Licenses and security advisories are clean (`cargo deny`). *slow* | `check:deny` |
| Commit subjects (and PR titles, which become the squashed subject) are Conventional Commits of at most 72 characters. | `check:conventional-commits` |
| No session links or session trailers in commits. | `check:no-session-links` |
| The layout below names every crate. | `check:layout-doc` |
| This table and `cargo xtask check` list the same checks. | `check:rules-doc` |
| Every source file (`.rs`, `.ts`, `.tsx`, `.js`, `.mjs`, `.css`, `.py`) starts with the `SPDX-License-Identifier: GPL-3.0-only` and `Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot` comment lines (see NOTICE and TRADEMARKS.md). | `check:copyright-headers` |

Not automated, still required:

- Public doc comments state behaviour and edge cases (the lint only checks
  that they exist).
- One focused change per pull request; CI green and the change reviewed
  before it is merged into `main`. A red `main` is fixed first.
- Changes to shared crates are also tried in the packaged app (`cd app &&
  npm run tauri build -- --bundles deb`) before merging.

## Commits and pull requests

- Conventional Commits (`feat(core): ...`, `fix: ...`, `test: ...`, `ci: ...`).
- Keep messages short: a subject line, plus a body only when the *why* is
  not obvious from the diff. No repeated summaries, no session links.

## Architecture rules

- `crates/serialbot-core` holds all device logic and must not depend on any UI.
  The app, the MCP server and the CLI only talk to devices through it.
- Serial ports are owned by the session manager; windows and MCP are views.
- Hardware access sits behind the `Transport` trait so logic is testable
  with mock transports; real-port tests use virtual ports (socat/com0com).
- MCP tools are a small curated set; everything else goes through
  `command.list` / `command.call`.
- Keep the core primitive: chip-specific flows (flashing, resets) belong in
  user Skills, not in the tool.

## Layout

- `crates/serialbot-core` — log buffer, transports, sessions (Rust)
  - `src/session.rs` — one open connection: reader thread, log, `wait_for`
  - `src/manager.rs` — owns all sessions; refuses to open a port twice
  - `src/transport/mock.rs` — in-memory device for tests of everything above transports
  - `tests/serial_socat.rs` — real serial code over socat virtual ports (Linux only)
- `crates/serialbot-jobs` — background jobs per session, shared by the app and the CLI:
  `filesend.rs` / `repeat.rs` / `export.rs` (file send, timed send, save/capture); the app
  stops them when a session closes
- `crates/serialbot-cli` — `serialbot-cli`, the terminal front end
  - `src/lib.rs` — `run(args, Context)`: commands get output, language and devices passed in
  - `src/commands/` — one file per subcommand (`ports`, `send`, `monitor`, `signal`, `open`, `run`,
    `mcp`: `mcp serve` asks in the terminal, `mcp stdio` bridges to it or the app)
  - `src/config.rs` — `cli.json` (MCP token and port) next to the app's settings
  - `src/script.rs` — `run` / `/run` scripts: text steps or the app's command export
  - `src/term/` — the interactive terminal: `keys.rs` (minicom keys, pure), `editor.rs`
    (input box), `feed.rs` (entries to lines), `screen.rs` (redraws only the bottom rows),
    `console.rs` (one session: lines, commands, jobs, reconnecting; shared with `tui`),
    `app.rs` (`open`: a console above the input box; tested with the `vt100` emulator)
  - `src/tui/` — the full-screen terminal: `app.rs` (sessions, up to four resizable panes,
    focus, one input box, the app's command groups; tested with mock devices and ratatui's
    `TestBackend`), `view.rs` (drawing),
    `sgr.rs` (color sequences to styles, wrapping), `plot.rs` (waveform: numbers from lines,
    six curves told apart by color and point shape), `ai.rs` (`--mcp`: the AI panel and its
    y/s/n answers; the server comes from `commands/mcp.rs`)
  - `src/i18n.rs` — every message and help text in English and Chinese
  - `tests/e2e.rs` — the real binary against socat virtual ports (Linux only)
  - `tests/interactive.rs` — `open` in a real pseudo terminal (`expectrl`) against socat
  - `tests/mcp.rs` — the real binary: `mcp serve` reached through `mcp stdio` (Linux only)
- `crates/serialbot-mcp` — MCP server (rmcp, Streamable HTTP at `/mcp`)
  - `src/tools.rs` — the curated tools; all device work goes through `SessionManager`
  - `src/lib.rs` — `router`/`serve`: bearer token, Host allowlist, Origin rejected
  - `src/server.rs` — `RunningServer`: bind local or LAN, port fallback, stop
  - `src/approval.rs` — `Approvals`: the queue of agent actions waiting for the user
  - `tests/http.rs` — real MCP client over HTTP against mock devices
- `app/` — desktop app (Tauri 2 + React/TypeScript, Vite, Vitest)
  - `src-tauri/src/commands.rs` — everything the UI can ask for, tested with mock devices;
    `lib.rs` only forwards IPC calls to it
  - `src-tauri/src/mcp.rs` — embedded MCP server on the same `SessionManager`
    (settings in `mcp.json` via `settings.rs`, status events to every window)
  - `src-tauri/src/watch.rs` — change detection; the window gets `sessions-changed` /
    `log-changed` events and reads new entries itself (no IPC polling)
  - `src/terminal.ts` — log entries to display lines (text/hex, line breaks, UTF-8)
  - `src/commands.ts` — saved command groups (local storage) and the in-order runner
  - `src/App.tsx` + `src/components/` — UI; tests use a fake `Backend`
  - Frontend gate: `npm run typecheck`, `npm run coverage` (≥ 85% lines), `npm run build`
- `xtask` — `cargo xtask check|policy|commit-msg|hooks`: the quality gates above
- `.github/workflows/ci.yml` — fmt, clippy, tests, MSRV, docs, coverage gate,
  license/advisory checks. PRs run on Linux x64; the Windows/macOS/Linux x64
  and arm64 matrix runs on `main`, weekly and on demand (Actions minutes for
  macOS/Windows are expensive), and so does the Linux arm64 packaging check.
  A red `main` run is fixed before anything else is merged.

## Releases

- `.github/workflows/release.yml` builds Windows (NSIS installer + portable
  exe), macOS (universal .dmg/.app, ad-hoc signed, not notarized) and Linux
  (.deb, .AppImage, x64 and arm64) and publishes them on the GitHub release page.
- Release by pushing a tag from `main` (`git tag v0.1.0-alpha.2 && git push
  origin v0.1.0-alpha.2`) or with Actions → Release → "Run workflow" on
  `main`, entering the tag. Tags with `-alpha`/`-beta`/`-rc` are pre-releases.
  The notes come from `.github/release-notes.md`.
- Check locally on Linux first: `cd app && npm run tauri build -- --bundles deb`.

## Testing on real hardware (done locally, not in CI)

- macOS: PTYs reject the `IOSSIOSPEED` baud ioctl, so serial I/O on macOS
  must be checked with a real USB adapter (CH340 / CP2102 / FT232).
- Windows: virtual-port tests need com0com; until then use a real adapter.
- Checklist per release: open/close, 115200 and 921600 round trip, busy
  detection, unplug while reading, DTR/RTS reset of an ESP32.
