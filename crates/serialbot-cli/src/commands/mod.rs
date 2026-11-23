//! One module per subcommand. Each takes its parsed arguments and an
//! [`crate::output::Output`], and talks to devices only through the core.

pub mod monitor;
pub mod open;
pub mod ports;
pub mod run;
pub mod send;
pub mod signal;
