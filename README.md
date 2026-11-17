# serialx

> Working codename. The product name will be chosen before the public beta.

A lightweight, install-free, cross-platform (Windows / macOS / Linux) serial
debugging assistant. An AI agent — built in, or any MCP client such as
Claude Code — can drive the ports and read back timestamped logs, so it can
close the loop on embedded debugging.

**Status:** early development (M0). Nothing usable yet.

## Development

Requires the stable Rust toolchain (MSRV 1.85).

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [CLAUDE.md](CLAUDE.md) for the quality rules every change must meet.

## License

The core is licensed under GPL-3.0-only. Plugin SDKs and device-side
libraries will be MIT-licensed.
