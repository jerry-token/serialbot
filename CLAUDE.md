# serialx — working rules

serialx (codename; product name TBD) is a lightweight, cross-platform serial
debugging assistant whose AI agent ("Bo") and MCP server can drive serial
ports. Plan of record: the "AI 串口助手 · 开工前计划" doc.

## Quality bar (non-negotiable)

- Every change ships with tests written in the same change. Bug fixes start
  with a failing test.
- Before every commit, all of these must pass locally:
  `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Line coverage of the workspace must stay at or above 85% (CI enforces it
  with `cargo llvm-cov`).
- No `unwrap()`/`expect()` outside tests; no `unsafe` (forbidden workspace-wide).
- Public items carry doc comments that state behaviour and edge cases.
- One focused change per pull request; CI must be green and the change
  reviewed before it is merged into `main`.

## Architecture rules

- `crates/serialx-core` holds all device logic and must not depend on any UI.
  The app, the MCP server and the CLI only talk to devices through it.
- Serial ports are owned by the session manager; windows and MCP are views.
- Hardware access sits behind the `Transport` trait so logic is testable
  with mock transports; real-port tests use virtual ports (socat/com0com).
- MCP tools are a small curated set; everything else goes through
  `command.list` / `command.call`.
- Keep the core primitive: chip-specific flows (flashing, resets) belong in
  user Skills, not in the tool.

## Layout

- `crates/serialx-core` — log buffer, transports, sessions (Rust)
  - `src/transport/mock.rs` — in-memory device for tests of everything above transports
  - `tests/serial_socat.rs` — real serial code over socat virtual ports (Linux only)

## Testing on real hardware (done locally, not in CI)

- macOS: PTYs reject the `IOSSIOSPEED` baud ioctl, so serial I/O on macOS
  must be checked with a real USB adapter (CH340 / CP2102 / FT232).
- Windows: virtual-port tests need com0com; until then use a real adapter.
- Checklist per release: open/close, 115200 and 921600 round trip, busy
  detection, unplug while reading, DTR/RTS reset of an ESP32.
- `.github/workflows/ci.yml` — fmt, clippy, tests on Windows/macOS/Linux,
  MSRV, docs, coverage gate, license/advisory checks
