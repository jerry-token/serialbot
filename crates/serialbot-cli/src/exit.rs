//! Process exit codes. Scripts tell outcomes apart by them, so they never
//! change meaning between versions.

/// The exit code of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    /// Done as asked.
    Ok = 0,
    /// What was waited for did not come in time (`--expect`, `--until`).
    Timeout = 1,
    /// Bad arguments.
    Usage = 2,
    /// The port could not be opened: missing, busy or not permitted.
    Busy = 3,
    /// The device went away or failed while in use.
    Disconnected = 4,
}
