//! The `serialbot-cli` program: the real terminal and serial ports around
//! [`serialbot_cli::run`].

use std::io::IsTerminal as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serialbot_cli::i18n::Lang;
use serialbot_cli::{Context, run};
use serialbot_core::SessionManager;

fn main() {
    let stdout = std::io::stdout();
    let out_is_terminal = stdout.is_terminal();
    let interactive = out_is_terminal && std::io::stdin().is_terminal();
    // Not locked for the whole run: `mcp stdio` writes to stdout from
    // another thread.
    let mut out = stdout;
    let mut err = std::io::stderr();
    let manager = Arc::new(SessionManager::with_serial_ports());
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    // Without a handler Ctrl+C would kill the process before the port is
    // closed and the log file finished; a second Ctrl+C still kills it.
    let _ = ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::Relaxed) {
            std::process::exit(130);
        }
    });
    let mut ctx = Context {
        out: &mut out,
        err: &mut err,
        out_is_terminal,
        no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        lang: Lang::from_env(),
        manager: Arc::clone(&manager),
        list_ports: Arc::new(serialbot_core::transport::serial::list_ports),
        interrupted,
        utc_offset_min: jiff::Zoned::now().offset().seconds() / 60,
        stdin: Some(Box::new(std::io::stdin())),
        interactive,
        config_dir: serialbot_cli::config::config_dir(),
    };
    let code = run(std::env::args_os(), &mut ctx);
    manager.close_all();
    std::process::exit(i32::from(code));
}
