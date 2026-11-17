//! Standalone MCP server over the machine's real serial ports, for trying
//! serialx with MCP clients before the desktop app exists.
//!
//! Listens on 127.0.0.1 only. Usage: `serialx-mcp [--port N] [--token T]`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool reports to the terminal"
)]

use std::sync::Arc;

use serialx_core::SessionManager;
use serialx_core::transport::serial::list_ports;
use serialx_mcp::{DEFAULT_PORT, MCP_PATH, McpOptions, SerialTools, generate_token};
use tokio_util::sync::CancellationToken;

const USAGE: &str = "usage: serialx-mcp [--port N] [--token T]";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    port: u16,
    token: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        port: DEFAULT_PORT,
        token: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let value = args.next().ok_or("--port needs a value")?;
                parsed.port = value
                    .parse()
                    .map_err(|_| format!("invalid port {value:?}"))?;
            }
            "--token" => parsed.token = Some(args.next().ok_or("--token needs a value")?),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    Ok(parsed)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1))?;
    let token = match args.token {
        Some(token) => token,
        None => generate_token().map_err(|e| format!("cannot generate a token: {e}"))?,
    };
    let options = McpOptions::new(token).map_err(|e| e.to_string())?;
    let manager = Arc::new(SessionManager::with_serial_ports());
    let tools = SerialTools::new(Arc::clone(&manager), Arc::new(list_ports));
    let shutdown = CancellationToken::new();
    let app = serialx_mcp::router(tools, &options, &shutdown);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port))
        .await
        .map_err(|e| format!("cannot listen on 127.0.0.1:{}: {e}", args.port))?;
    let url = format!("http://127.0.0.1:{}{MCP_PATH}", args.port);
    println!("serialx MCP server on {url}");
    println!("token: {}", options.token());
    println!(
        "Claude Code: claude mcp add --transport http serialx {url} --header \"Authorization: Bearer {}\"",
        options.token()
    );
    let stop = shutdown.clone();
    tokio::spawn(async move {
        // If Ctrl-C cannot be watched, the server simply runs until killed.
        if tokio::signal::ctrl_c().await.is_ok() {
            stop.cancel();
        }
    });
    let served = serialx_mcp::serve(listener, app, shutdown).await;
    manager.close_all();
    served.map_err(|e| format!("server error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn defaults() {
        assert_eq!(
            parse(&[]).unwrap(),
            Args {
                port: DEFAULT_PORT,
                token: None
            }
        );
    }

    #[test]
    fn port_and_token() {
        assert_eq!(
            parse(&["--port", "9700", "--token", "t"]).unwrap(),
            Args {
                port: 9700,
                token: Some("t".into())
            }
        );
    }

    #[test]
    fn errors() {
        assert!(parse(&["--port"]).unwrap_err().contains("needs a value"));
        assert!(
            parse(&["--port", "70000"])
                .unwrap_err()
                .contains("invalid port")
        );
        assert!(parse(&["--token"]).unwrap_err().contains("needs a value"));
        assert!(
            parse(&["--bogus"])
                .unwrap_err()
                .contains("unknown argument")
        );
        assert_eq!(parse(&["--help"]).unwrap_err(), USAGE);
    }
}
