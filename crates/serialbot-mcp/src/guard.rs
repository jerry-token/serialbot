//! Asking the user before an agent changes anything.
//!
//! Tools that send data, change lines or settings, or open and close ports
//! describe what they are about to do as an [`Action`] and wait for the
//! [`Guard`] to approve it. Reading never asks. Without a guard
//! ([`AllowAll`]) every action goes ahead, which suits tests and
//! `serialbot-cli mcp serve --approve all`.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;

use serialbot_core::SessionId;

/// Longest data preview in [`Action::detail`], in bytes.
pub const PREVIEW_BYTES: usize = 48;

/// What an agent is about to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Tool name, for example `io.write`; `command.call` adds the command
    /// (`command.call io.break`).
    pub tool: String,
    /// Session the action targets; `None` when opening a new one.
    pub session: Option<SessionId>,
    /// Port path or endpoint the action touches.
    pub port: String,
    /// One line for the user: the data sent (see [`preview`]) or what changes.
    pub detail: String,
}

/// Outcome of [`Guard::approve`]: `Err` carries the reason given to the agent.
pub type Approval<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Decides whether an agent may carry out an [`Action`].
///
/// Tools await the returned future before acting, so a guard may take as
/// long as the user needs; dropping the future (the agent gave up)
/// abandons the action.
pub trait Guard: Send + Sync {
    /// Resolves to `Ok(())` to go ahead, or `Err(reason)` to refuse.
    fn approve(&self, action: Action) -> Approval<'_>;
}

/// Guard that approves everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl Guard for AllowAll {
    fn approve(&self, _action: Action) -> Approval<'_> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Describes outgoing bytes in one line: quoted text with `\r`, `\n`, `\t`
/// and `\xNN` escapes when they are mostly printable UTF-8, HEX otherwise;
/// the first [`PREVIEW_BYTES`] bytes followed by `…`, then the length.
pub fn preview(data: &[u8]) -> String {
    if data.is_empty() {
        return "0 bytes".into();
    }
    let mut shown = &data[..data.len().min(PREVIEW_BYTES)];
    // Do not cut a character in half at the preview limit.
    if shown.len() < data.len()
        && let Err(e) = std::str::from_utf8(shown)
        && e.error_len().is_none()
    {
        shown = &shown[..e.valid_up_to()];
    }
    let more = if data.len() > shown.len() { "…" } else { "" };
    let unit = if data.len() == 1 { "byte" } else { "bytes" };
    let (mut total, mut printable) = (0, 0);
    for chunk in shown.utf8_chunks() {
        for c in chunk.valid().chars() {
            total += 1;
            if !c.is_control() || matches!(c, '\r' | '\n' | '\t') {
                printable += 1;
            }
        }
        total += chunk.invalid().len();
    }
    let body = if printable * 4 >= total * 3 {
        let mut quoted = String::from("\"");
        for chunk in shown.utf8_chunks() {
            for c in chunk.valid().chars() {
                match c {
                    '\r' => quoted.push_str("\\r"),
                    '\n' => quoted.push_str("\\n"),
                    '\t' => quoted.push_str("\\t"),
                    '"' => quoted.push_str("\\\""),
                    '\\' => quoted.push_str("\\\\"),
                    c if c.is_control() => escape(&mut quoted, c.to_string().as_bytes()),
                    c => quoted.push(c),
                }
            }
            escape(&mut quoted, chunk.invalid());
        }
        quoted.push('"');
        quoted
    } else {
        shown
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!("{body}{more} ({} {unit})", data.len())
}

/// Appends `bytes` as `\xNN` escapes.
fn escape(out: &mut String, bytes: &[u8]) {
    for b in bytes {
        let _ = write!(out, "\\x{b:02x}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_text_with_escapes() {
        assert_eq!(preview(b"AT+GMR\r\n"), r#""AT+GMR\r\n" (8 bytes)"#);
        assert_eq!(preview(b"say \"hi\"\\"), r#""say \"hi\"\\" (9 bytes)"#);
        assert_eq!(preview("温度\t".as_bytes()), "\"温度\\t\" (7 bytes)");
        assert_eq!(preview(b"ab\x01cdefgh"), r#""ab\x01cdefgh" (9 bytes)"#);
    }

    #[test]
    fn previews_binary_as_hex() {
        assert_eq!(preview(&[0xff]), "ff (1 byte)");
        assert_eq!(preview(&[0x01, 0x02, 0x41]), "01 02 41 (3 bytes)");
        assert_eq!(preview(&[]), "0 bytes");
    }

    #[test]
    fn cuts_long_data() {
        let long = vec![b'a'; PREVIEW_BYTES + 10];
        let p = preview(&long);
        assert!(
            p.starts_with(&format!("\"{}\"…", "a".repeat(PREVIEW_BYTES))),
            "{p}"
        );
        assert!(p.ends_with("(58 bytes)"), "{p}");
        let mut cut = vec![b'a'; PREVIEW_BYTES - 1];
        cut.extend("温".as_bytes());
        let p = preview(&cut);
        assert!(p.ends_with("a\"… (50 bytes)"), "{p}");
        assert_eq!(
            preview(b"OK\xffREADY\r\n"),
            r#""OK\xffREADY\r\n" (10 bytes)"#
        );
    }

    #[tokio::test]
    async fn allow_all_approves() {
        let action = Action {
            tool: "io.write".into(),
            session: Some(SessionId(1)),
            port: "COM3".into(),
            detail: preview(b"x"),
        };
        assert_eq!(AllowAll.approve(action).await, Ok(()));
    }
}
