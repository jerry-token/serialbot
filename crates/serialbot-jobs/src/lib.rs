//! Background jobs that run on a session for as long as a front end wants
//! them: sending a file in chunks ([`filesend`]), sending the same data on
//! a timer ([`repeat`]), and saving or capturing the log to a file
//! ([`export`]).
//!
//! Each job runs on its own thread, so the desktop app and the command-line
//! tool stay responsive and can show progress or stop it. None of them
//! depends on a user interface.

pub mod export;
pub mod filesend;
pub mod repeat;

#[cfg(test)]
pub(crate) mod test_dir {
    //! A temporary directory removed when the test ends.
    #![allow(clippy::unwrap_used, reason = "test helper")]

    use std::path::PathBuf;

    /// A fresh directory under the system temp dir.
    pub(crate) struct TempDir(pub(crate) PathBuf);

    impl TempDir {
        /// `name` must be unique among the tests; the process id keeps
        /// parallel test runs apart. Only characters valid on every OS.
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("serialbot-jobs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
