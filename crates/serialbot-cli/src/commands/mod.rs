//! One module per subcommand. Each takes its parsed arguments and an
//! [`crate::output::Output`], and talks to devices only through the core.

pub mod ports;
