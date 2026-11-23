// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Web links the help page may open in the system browser or mail app.
//!
//! The UI cannot open arbitrary URLs: only the fixed set below is accepted,
//! so a compromised page cannot launch other programs through this path.

/// The project website.
pub const WEBSITE: &str = "https://serialbot.aiany.cc";
/// The source code and releases.
pub const REPOSITORY: &str = "https://github.com/jerry-token/serialbot";
/// Where bugs and ideas are reported.
pub const ISSUES: &str = "https://github.com/jerry-token/serialbot/issues";
/// The release page with all downloads.
pub const RELEASES: &str = "https://github.com/jerry-token/serialbot/releases";
/// Mail to the maintainer.
pub const SUPPORT_MAIL: &str = "mailto:support@aiany.cc";

/// Every link the help page may open.
pub const ALLOWED: [&str; 5] = [WEBSITE, REPOSITORY, ISSUES, RELEASES, SUPPORT_MAIL];

/// Returns `url` if it is one of [`ALLOWED`] (exact match, no prefixes or
/// added paths), otherwise an error naming it.
pub fn checked(url: &str) -> Result<&'static str, String> {
    ALLOWED
        .iter()
        .find(|allowed| **allowed == url)
        .copied()
        .ok_or_else(|| format!("not a SerialBot link: {url}"))
}

/// Opens an allowed link with the system's default handler (browser or
/// mail app) without waiting for it.
pub fn open(url: &str) -> Result<(), String> {
    let url = checked(url)?;
    open::that_detached(url).map_err(|e| format!("could not open {url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exactly_the_listed_links() {
        for url in ALLOWED {
            assert_eq!(checked(url), Ok(url));
        }
    }

    #[test]
    fn refuses_anything_else() {
        for url in [
            "",
            "https://serialbot.aiany.cc/../evil",
            "https://serialbot.aiany.cc.evil.example",
            "https://github.com/jerry-token/serialbot/issues/new?x=1",
            "file:///etc/passwd",
            "calc.exe",
        ] {
            let err = checked(url).unwrap_err();
            assert!(err.contains("not a SerialBot link"), "{err}");
        }
        assert!(open("file:///etc/passwd").is_err());
    }
}
