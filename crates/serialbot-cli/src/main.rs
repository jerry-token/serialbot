//! The `serialbot-cli` program: the real terminal and serial ports around
//! [`serialbot_cli::run`].

use std::io::IsTerminal as _;
use std::sync::Arc;

use serialbot_cli::i18n::Lang;
use serialbot_cli::{Context, run};
use serialbot_core::SessionManager;

fn main() {
    let stdout = std::io::stdout();
    let out_is_terminal = stdout.is_terminal();
    let mut out = stdout.lock();
    let mut err = std::io::stderr().lock();
    let manager = Arc::new(SessionManager::with_serial_ports());
    let mut ctx = Context {
        out: &mut out,
        err: &mut err,
        out_is_terminal,
        no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
        lang: Lang::from_env(),
        manager: Arc::clone(&manager),
        list_ports: Arc::new(serialbot_core::transport::serial::list_ports),
    };
    let code = run(std::env::args_os(), &mut ctx);
    manager.close_all();
    std::process::exit(i32::from(code));
}
