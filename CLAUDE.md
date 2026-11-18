# SerialBot — working rules

SerialBot (`serialbot` for binaries, crates and packages) is a lightweight, cross-platform serial
debugging assistant whose AI agent ("Bo") and MCP server can drive serial
ports. Plan of record: the "AI 串口助手 · 开工前计划" doc.

## Quality bar (non-negotiable)

- Every change ships with tests written in the same change. Bug fixes start
  with a failing test.
- Before every commit, all of these must pass locally:
  `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and for frontend changes the frontend gate
  below.
- Line coverage of the workspace must stay at or above 85% (CI enforces it
  with `cargo llvm-cov`).
- No `unwrap()`/`expect()` outside tests; no `unsafe` (forbidden workspace-wide).
- Public items carry doc comments that state behaviour and edge cases.
- One focused change per pull request; CI must be green and the change
  reviewed before it is merged into `main`.

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
- `crates/serialbot-mcp` — MCP server (rmcp, Streamable HTTP at `/mcp`)
  - `src/tools.rs` — the curated tools; all device work goes through `SessionManager`
  - `src/lib.rs` — `router`/`serve`: bearer token, Host allowlist, Origin rejected
  - `src/main.rs` — `serialbot-mcp` binary on 127.0.0.1 for trying it before the app exists
  - `tests/http.rs` — real MCP client over HTTP against mock devices
- `app/` — desktop app (Tauri 2 + React/TypeScript, Vite, Vitest)
  - `src-tauri/src/commands.rs` — everything the UI can ask for, tested with mock devices;
    `lib.rs` only forwards IPC calls to it
  - `src-tauri/src/mcp.rs` — embedded MCP server on the same `SessionManager`
    (127.0.0.1, port fallback, settings in `mcp.json` via `settings.rs`)
  - `src-tauri/src/filesend.rs` / `repeat.rs` / `export.rs` — background jobs per session
    (file send, timed send, save/capture), stopped when the session closes
  - `src-tauri/src/watch.rs` — change detection; the window gets `sessions-changed` /
    `log-changed` events and reads new entries itself (no IPC polling)
  - `src/terminal.ts` — log entries to display lines (text/hex, line breaks, UTF-8)
  - `src/commands.ts` — saved command groups (local storage) and the in-order runner
  - `src/App.tsx` + `src/components/` — UI; tests use a fake `Backend`
  - Frontend gate: `npm run typecheck`, `npm run coverage` (≥ 85% lines), `npm run build`
- `.github/workflows/ci.yml` — fmt, clippy, tests, MSRV, docs, coverage gate,
  license/advisory checks. PRs run on Linux; the Windows/macOS/Linux matrix
  runs on `main`, weekly and on demand (Actions minutes for macOS/Windows
  are expensive). A red `main` run is fixed before anything else is merged.

## Releases

- `.github/workflows/release.yml` builds Windows (NSIS installer + portable
  exe), macOS (universal .dmg/.app, ad-hoc signed, not notarized) and Linux
  (.deb, .AppImage) and publishes them on the GitHub release page.
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
