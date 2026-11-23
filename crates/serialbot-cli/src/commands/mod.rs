//! One module per subcommand. Each takes its parsed arguments and an
//! [`crate::output::Output`], and talks to devices only through the core.

pub mod monitor;
pub mod ports;
pub mod send;
