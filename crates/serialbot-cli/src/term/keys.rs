// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! What each key does in the interactive terminal, as a pure state
//! machine: keys in, [`Action`]s out. No terminal and no port are touched
//! here, so every key is tested.
//!
//! Keys follow minicom: `Ctrl+A` then a letter runs a local command (the
//! same letters as minicom; see [`Local`]), and `Ctrl+A Ctrl+A` sends
//! `Ctrl+A` itself. The prefix can be another `Ctrl+<letter>` for people
//! whose device shell uses `Ctrl+A`.
//!
//! In line mode keys edit the input box and Enter sends the line; a line
//! starting with `/` is a command ([`Slash`]), `//` sends a literal `/`.
//! In raw mode every key goes to the device at once, as in minicom.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::editor::Editor;

/// How keys are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// An input box; Enter sends the line.
    Line,
    /// Every key goes to the device at once (shells, vim, top).
    Raw,
}

/// A local command after the `Ctrl+A` prefix, by minicom's letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Local {
    /// `Z`: key help.
    Help,
    /// `X`: quit and close the port (`Q`: quit without touching DTR/RTS).
    Quit {
        /// Whether closing may reset the modem lines (`X`) or not (`Q`).
        reset: bool,
    },
    /// `C`: clear the screen.
    Clear,
    /// `E`: local echo on/off.
    ToggleEcho,
    /// `F`: send a BREAK.
    Break,
    /// `H`: drop DTR for a moment ("hang up").
    Hangup,
    /// `L`: start or stop saving to a file.
    ToggleCapture,
    /// `N`: cycle timestamps (none, time, time since the previous line).
    CycleTimestamp,
    /// `P`: baud rate, parity, bits.
    PortSettings,
    /// `S`: send a file (in chunks, with progress).
    SendFile,
    /// `Y`: send a file as if typed.
    TypeFile,
    /// `G`: run a script.
    RunScript,
    /// `W`: line wrap on/off.
    ToggleWrap,
    /// `A`: add a line feed to received carriage returns.
    ToggleAddLf,
    /// `U`: add a carriage return to received line feeds.
    ToggleAddCr,
    /// `B`: scrollback and search.
    Scrollback,
    /// `T`: display options (hex, encoding, highlights).
    Display,
    /// `O`: settings, saved as defaults.
    Config,
    /// `J`: a shell, returning to the terminal afterwards.
    Shell,
    /// `1`: flip DTR.
    ToggleDtr,
    /// `2`: flip RTS.
    ToggleRts,
    /// `3`: run the reset sequence.
    Reset,
    /// `/`: open the command palette.
    Palette,
    /// `Ctrl+A` + an arrow: focus the pane that way (`tui`).
    Focus(Towards),
    /// `|`: panes side by side (`tui`).
    SplitSide,
    /// `-`: panes one above the other (`tui`).
    SplitStack,
    /// `V`: the plot on/off (`tui`).
    TogglePlot,
    /// A minicom key for modems that SerialBot does not offer
    /// (`D` dial, `M` init string, `K` kermit, `R` receive, `I` cursor keys).
    Unsupported(char),
    /// A key with no meaning after the prefix.
    Unknown(char),
}

/// A direction of focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Towards {
    /// ← or ↑: the previous pane.
    Previous,
    /// → or ↓: the next pane.
    Next,
}

/// A command typed in the input box after `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slash {
    /// `/baud 921600`.
    Baud(u32),
    /// `/hex`: show received data as hex or text.
    Hex,
    /// `/time`: cycle timestamps.
    Time,
    /// `/log [file]`: start saving (to the file) or stop.
    Log(Option<String>),
    /// `/send-file <file>`.
    SendFile(String),
    /// `/run <script>`.
    Run(String),
    /// `/stop`: stop a running script or file send.
    Stop,
    /// `/raw`: switch to raw mode.
    Raw,
    /// `/echo`: local echo on/off.
    Echo,
    /// `/dtr`, `/rts`: flip the line.
    Dtr,
    /// See [`Slash::Dtr`].
    Rts,
    /// `/break`.
    Break,
    /// `/reset`: the reset sequence.
    Reset,
    /// `/clear`.
    Clear,
    /// `/help`.
    Help,
    /// `/quit`.
    Quit,
    /// `/open <port> [baud]`: another session (`tui`).
    Open {
        /// Port name or network address.
        port: String,
        /// Baud rate; the one `tui` started with if left out.
        baud: Option<u32>,
    },
    /// `/close`: close the focused session's port; its log stays (`tui`).
    Close,
    /// `/plot [pattern | auto | clear | color <curve> <color>]`: the
    /// waveform on/off, or how it finds numbers (`tui`).
    Plot(Option<String>),
    /// `/groups [file]`: load the app's command export, or show/hide it
    /// (`tui`).
    Groups(Option<String>),
    /// `/group <name or number>`: the group shown (`tui`).
    Group(String),
    /// `/cmd <number>` sends that command of the group; `/cmd all` runs the
    /// group in order (`tui`).
    Cmd(String),
    /// Anything else, or missing/invalid arguments: the text after `/`.
    Invalid(String),
}

/// The commands of the palette: name, argument, for completion and help.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("baud", "<rate>"),
    ("break", ""),
    ("clear", ""),
    ("close", ""),
    ("dtr", ""),
    ("echo", ""),
    ("help", ""),
    ("hex", ""),
    ("log", "[file]"),
    ("open", "<port> [baud]"),
    ("plot", "[pattern|auto|clear|color <curve> <color>]"),
    ("groups", "[file]"),
    ("group", "<name>"),
    ("cmd", "<number|all>"),
    ("quit", ""),
    ("raw", ""),
    ("reset", ""),
    ("rts", ""),
    ("run", "<script>"),
    ("send-file", "<file>"),
    ("stop", ""),
    ("time", ""),
];

impl Slash {
    /// Parses the text after `/`.
    pub fn parse(text: &str) -> Self {
        let mut words = text.split_whitespace();
        let name = words.next().unwrap_or_default().to_ascii_lowercase();
        let rest: Vec<&str> = words.collect();
        let arg = (!rest.is_empty()).then(|| rest.join(" "));
        let invalid = || Self::Invalid(text.trim().to_owned());
        match (name.as_str(), arg) {
            ("baud", Some(a)) => a
                .parse()
                .ok()
                .filter(|b| *b > 0)
                .map_or_else(invalid, Self::Baud),
            ("hex", None) => Self::Hex,
            ("time", None) => Self::Time,
            ("log", a) => Self::Log(a),
            ("send-file", Some(a)) => Self::SendFile(a),
            ("run", Some(a)) => Self::Run(a),
            ("raw", None) => Self::Raw,
            ("stop", None) => Self::Stop,
            ("echo", None) => Self::Echo,
            ("dtr", None) => Self::Dtr,
            ("rts", None) => Self::Rts,
            ("break", None) => Self::Break,
            ("reset", None) => Self::Reset,
            ("clear", None) => Self::Clear,
            ("help", None) => Self::Help,
            ("quit" | "exit", None) => Self::Quit,
            ("close", None) => Self::Close,
            ("groups", a) => Self::Groups(a),
            ("group", Some(a)) => Self::Group(a),
            ("cmd", Some(a)) => Self::Cmd(a),
            // The pattern as typed: spaces in a regex matter.
            ("plot", a) => Self::Plot(a.map(|_| {
                let typed = text.trim_start();
                typed[typed.find(char::is_whitespace).unwrap_or(typed.len())..]
                    .trim()
                    .to_owned()
            })),
            ("open", Some(_)) => match rest.as_slice() {
                [port] => Self::Open {
                    port: (*port).to_owned(),
                    baud: None,
                },
                [port, baud] => baud
                    .parse()
                    .ok()
                    .filter(|b| *b > 0)
                    .map_or_else(invalid, |b| Self::Open {
                        port: (*port).to_owned(),
                        baud: Some(b),
                    }),
                _ => invalid(),
            },
            _ => invalid(),
        }
    }
}

/// What a key asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send these bytes now (raw mode, control keys).
    Send(Vec<u8>),
    /// Send this line (encoded and with the line ending added by the
    /// caller).
    SendLine(String),
    /// Run a local command.
    Local(Local),
    /// Run a palette command.
    Command(Slash),
    /// The input box changed; draw it again.
    Edited,
}

/// A history search in progress (Ctrl+R).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Search {
    /// What is searched for.
    pub needle: String,
    /// The history entry found, if any.
    pub found: Option<usize>,
}

/// The key state of one interactive session.
#[derive(Debug, Clone)]
pub struct Keys {
    /// Line or raw mode.
    pub mode: Mode,
    /// The input box.
    pub editor: Editor,
    /// The prefix key's letter (`a` for `Ctrl+A`).
    pub escape: char,
    /// A history search, while one is open.
    pub search: Option<Search>,
    prefix: bool,
}

impl Keys {
    /// Line mode with the `Ctrl+<escape>` prefix.
    pub fn new(mode: Mode, escape: char) -> Self {
        Self {
            mode,
            editor: Editor::default(),
            escape: escape.to_ascii_lowercase(),
            search: None,
            prefix: false,
        }
    }

    /// Whether the prefix was pressed and the next key is a command.
    pub fn prefix_pending(&self) -> bool {
        self.prefix
    }

    /// Handles one key press. Releases and repeats of releases are ignored.
    pub fn key(&mut self, key: KeyEvent) -> Vec<Action> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.prefix {
            self.prefix = false;
            return self.after_prefix(key, ctrl);
        }
        if ctrl && key.code == KeyCode::Char(self.escape) {
            self.prefix = true;
            return Vec::new();
        }
        match self.mode {
            Mode::Raw => raw_bytes(key).map(Action::Send).into_iter().collect(),
            Mode::Line if self.search.is_some() => self.search_key(key, ctrl),
            Mode::Line => self.line_key(key, ctrl),
        }
    }

    /// Handles pasted text: inserted into the box, or sent in raw mode.
    pub fn paste(&mut self, text: &str) -> Vec<Action> {
        match self.mode {
            Mode::Raw => vec![Action::Send(text.as_bytes().to_vec())],
            Mode::Line => {
                self.editor.insert_str(text);
                vec![Action::Edited]
            }
        }
    }

    fn after_prefix(&mut self, key: KeyEvent, ctrl: bool) -> Vec<Action> {
        let c = match key.code {
            KeyCode::Char(c) => c,
            KeyCode::Left | KeyCode::Up => {
                return vec![Action::Local(Local::Focus(Towards::Previous))];
            }
            KeyCode::Right | KeyCode::Down => {
                return vec![Action::Local(Local::Focus(Towards::Next))];
            }
            // Esc or any other key cancels the prefix.
            _ => return Vec::new(),
        };
        if ctrl && c.to_ascii_lowercase() == self.escape {
            return vec![Action::Send(vec![ctrl_byte(self.escape)])];
        }
        let local = match c.to_ascii_lowercase() {
            'z' => Local::Help,
            'x' => Local::Quit { reset: true },
            'q' => Local::Quit { reset: false },
            'c' => Local::Clear,
            'e' => Local::ToggleEcho,
            'f' => Local::Break,
            'h' => Local::Hangup,
            'l' => Local::ToggleCapture,
            'n' => Local::CycleTimestamp,
            'p' => Local::PortSettings,
            's' => Local::SendFile,
            'y' => Local::TypeFile,
            'g' => Local::RunScript,
            'w' => Local::ToggleWrap,
            'a' => Local::ToggleAddLf,
            'u' => Local::ToggleAddCr,
            'b' => Local::Scrollback,
            't' => Local::Display,
            'o' => Local::Config,
            'j' => Local::Shell,
            '1' => Local::ToggleDtr,
            '2' => Local::ToggleRts,
            '3' => Local::Reset,
            '/' => Local::Palette,
            '|' | '\\' => Local::SplitSide,
            '-' => Local::SplitStack,
            'v' => Local::TogglePlot,
            other @ ('d' | 'm' | 'k' | 'r' | 'i') => Local::Unsupported(other),
            other => Local::Unknown(other),
        };
        if local == Local::Palette && self.mode == Mode::Line {
            self.editor.clear();
            self.editor.insert('/');
            return vec![Action::Edited];
        }
        vec![Action::Local(local)]
    }

    fn line_key(&mut self, key: KeyEvent, ctrl: bool) -> Vec<Action> {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let e = &mut self.editor;
        match key.code {
            KeyCode::Enter if shift || alt => e.insert('\n'),
            KeyCode::Enter => return vec![self.submit()],
            KeyCode::Char('c') if ctrl => {
                if e.is_empty() {
                    return vec![Action::Send(vec![0x03])];
                }
                e.clear();
            }
            KeyCode::Char('d') if ctrl && e.is_empty() => return vec![Action::Send(vec![0x04])],
            KeyCode::Char('d') if ctrl => e.delete(),
            KeyCode::Char('l') if ctrl => return vec![Action::Local(Local::Clear)],
            KeyCode::Char('u') if ctrl => e.delete_to_line_start(),
            KeyCode::Char('w') if ctrl => e.delete_word(),
            KeyCode::Char('a') if ctrl => e.home(),
            KeyCode::Char('e') if ctrl => e.end(),
            KeyCode::Char('r') if ctrl => {
                self.search = Some(Search::default());
            }
            KeyCode::Char(_) if ctrl || alt => return Vec::new(),
            KeyCode::Char(c) => e.insert(c),
            KeyCode::Tab => self.complete(),
            KeyCode::Backspace => e.backspace(),
            KeyCode::Delete => e.delete(),
            KeyCode::Left => e.left(),
            KeyCode::Right => e.right(),
            KeyCode::Home => e.home(),
            KeyCode::End => e.end(),
            KeyCode::Up => {
                if !e.history_up() {
                    return Vec::new();
                }
            }
            KeyCode::Down => {
                if !e.history_down() {
                    return Vec::new();
                }
            }
            KeyCode::Esc => e.clear(),
            _ => return Vec::new(),
        }
        vec![Action::Edited]
    }

    /// Enter: a palette command, or a line to send.
    fn submit(&mut self) -> Action {
        let line = self.editor.submit();
        match line.strip_prefix('/') {
            Some(rest) if rest.starts_with('/') => Action::SendLine(rest.to_owned()),
            Some(rest) => Action::Command(Slash::parse(rest)),
            None => Action::SendLine(line),
        }
    }

    /// Tab: completes a palette command name.
    fn complete(&mut self) {
        let text = self.editor.text();
        let Some(typed) = text.strip_prefix('/') else {
            return;
        };
        if typed.contains(char::is_whitespace) {
            return;
        }
        let matches: Vec<&str> = SLASH_COMMANDS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| name.starts_with(typed))
            .collect();
        let Some(first) = matches.first() else {
            return;
        };
        // The longest prefix all matches share; a space after a unique one.
        let common: String = first
            .chars()
            .enumerate()
            .take_while(|(i, c)| matches.iter().all(|m| m.chars().nth(*i) == Some(*c)))
            .map(|(_, c)| c)
            .collect();
        self.editor.clear();
        self.editor.insert('/');
        self.editor.insert_str(&common);
        if matches.len() == 1 {
            self.editor.insert(' ');
        }
    }

    fn search_key(&mut self, key: KeyEvent, ctrl: bool) -> Vec<Action> {
        let Some(search) = self.search.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Enter => {
                if let Some(found) = search.found {
                    self.editor.take_from_history(found);
                }
                self.search = None;
            }
            KeyCode::Esc => self.search = None,
            KeyCode::Char('c' | 'g') if ctrl => self.search = None,
            KeyCode::Char('r') if ctrl => {
                let before = search.found;
                if let Some((i, _)) = self.editor.search(&search.needle, before) {
                    search.found = Some(i);
                }
            }
            KeyCode::Backspace => {
                search.needle.pop();
                search.found = self.editor.search(&search.needle, None).map(|(i, _)| i);
            }
            KeyCode::Char(c) if !ctrl => {
                search.needle.push(c);
                search.found = self.editor.search(&search.needle, None).map(|(i, _)| i);
            }
            _ => return Vec::new(),
        }
        vec![Action::Edited]
    }
}

/// `Ctrl+<letter>` as a byte (`a` → 0x01).
fn ctrl_byte(letter: char) -> u8 {
    u8::try_from(letter.to_ascii_lowercase()).map_or(0, |b| b & 0x1f)
}

/// The bytes a key sends in raw mode, as a VT100/xterm terminal would.
pub fn raw_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let bytes: Vec<u8> = match key.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => vec![ctrl_byte(c)],
        KeyCode::Char(' ') if ctrl => vec![0x00],
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let text = c.encode_utf8(&mut buf).as_bytes().to_vec();
            if alt {
                [vec![0x1b], text].concat()
            } else {
                text
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + n - 1],
        KeyCode::F(n @ 5..=12) => {
            // xterm: F5=15, F6..F8=17..19, F9,F10=20,21, F11,F12=23,24.
            let code = [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)];
            format!("\x1b[{code}~").into_bytes()
        }
        _ => return None,
    };
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typing(keys: &mut Keys, text: &str) {
        for c in text.chars() {
            keys.key(press(KeyCode::Char(c)));
        }
    }

    fn line() -> Keys {
        Keys::new(Mode::Line, 'a')
    }

    #[test]
    fn every_minicom_letter_after_the_prefix() {
        let table = [
            ('z', Local::Help),
            ('x', Local::Quit { reset: true }),
            ('q', Local::Quit { reset: false }),
            ('c', Local::Clear),
            ('e', Local::ToggleEcho),
            ('f', Local::Break),
            ('h', Local::Hangup),
            ('l', Local::ToggleCapture),
            ('n', Local::CycleTimestamp),
            ('p', Local::PortSettings),
            ('s', Local::SendFile),
            ('y', Local::TypeFile),
            ('g', Local::RunScript),
            ('w', Local::ToggleWrap),
            ('a', Local::ToggleAddLf),
            ('u', Local::ToggleAddCr),
            ('b', Local::Scrollback),
            ('t', Local::Display),
            ('o', Local::Config),
            ('j', Local::Shell),
            ('1', Local::ToggleDtr),
            ('2', Local::ToggleRts),
            ('3', Local::Reset),
            ('d', Local::Unsupported('d')),
            ('m', Local::Unsupported('m')),
            ('k', Local::Unsupported('k')),
            ('r', Local::Unsupported('r')),
            ('i', Local::Unsupported('i')),
            ('v', Local::TogglePlot),
            ('-', Local::SplitStack),
            ('|', Local::SplitSide),
            ('\\', Local::SplitSide),
            ('`', Local::Unknown('`')),
        ];
        for mode in [Mode::Line, Mode::Raw] {
            for (letter, local) in table {
                let mut keys = Keys::new(mode, 'a');
                assert!(keys.key(ctrl('a')).is_empty());
                assert!(keys.prefix_pending());
                assert_eq!(
                    keys.key(press(KeyCode::Char(letter))),
                    [Action::Local(local)],
                    "{letter}"
                );
                assert!(!keys.prefix_pending());
                // Capitals and Ctrl+letter work too, as in minicom.
                keys.key(ctrl('a'));
                let upper = KeyEvent::new(
                    KeyCode::Char(letter.to_ascii_uppercase()),
                    KeyModifiers::SHIFT,
                );
                assert_eq!(keys.key(upper), [Action::Local(local)], "{letter}");
            }
        }
    }

    #[test]
    fn the_prefix_twice_sends_it_and_another_prefix_can_be_chosen() {
        let mut keys = line();
        keys.key(ctrl('a'));
        assert_eq!(keys.key(ctrl('a')), [Action::Send(vec![0x01])]);
        keys.key(ctrl('a'));
        assert!(keys.key(press(KeyCode::Esc)).is_empty(), "Esc cancels");
        assert!(!keys.prefix_pending());

        let mut keys = Keys::new(Mode::Raw, 'T');
        assert_eq!(
            keys.key(ctrl('a')),
            [Action::Send(vec![0x01])],
            "Ctrl+A is free"
        );
        keys.key(ctrl('t'));
        assert_eq!(
            keys.key(press(KeyCode::Char('x'))),
            [Action::Local(Local::Quit { reset: true })]
        );
        keys.key(ctrl('t'));
        assert_eq!(keys.key(ctrl('t')), [Action::Send(vec![0x14])]);
    }

    #[test]
    fn enter_sends_the_line_and_slash_runs_commands() {
        let mut keys = line();
        typing(&mut keys, "AT+GMR");
        assert_eq!(
            keys.key(press(KeyCode::Enter)),
            [Action::SendLine("AT+GMR".into())]
        );
        assert!(keys.editor.is_empty());
        assert_eq!(
            keys.key(press(KeyCode::Enter)),
            [Action::SendLine(String::new())],
            "an empty line"
        );
        typing(&mut keys, "/baud 921600");
        assert_eq!(
            keys.key(press(KeyCode::Enter)),
            [Action::Command(Slash::Baud(921_600))]
        );
        typing(&mut keys, "//etc");
        assert_eq!(
            keys.key(press(KeyCode::Enter)),
            [Action::SendLine("/etc".into())]
        );
        // Ctrl+A / opens the palette by typing the slash.
        keys.key(ctrl('a'));
        assert_eq!(keys.key(press(KeyCode::Char('/'))), [Action::Edited]);
        assert_eq!(keys.editor.text(), "/");
    }

    #[test]
    fn arrows_after_the_prefix_move_the_focus() {
        let mut keys = line();
        for (code, towards) in [
            (KeyCode::Left, Towards::Previous),
            (KeyCode::Up, Towards::Previous),
            (KeyCode::Right, Towards::Next),
            (KeyCode::Down, Towards::Next),
        ] {
            keys.key(ctrl('a'));
            assert_eq!(
                keys.key(press(code)),
                [Action::Local(Local::Focus(towards))]
            );
        }
        keys.key(ctrl('a'));
        assert!(keys.key(press(KeyCode::Esc)).is_empty());
        assert!(!keys.prefix_pending());
    }

    #[test]
    fn slash_commands_parse_their_arguments() {
        let cases = [
            ("baud 115200", Slash::Baud(115_200)),
            ("hex", Slash::Hex),
            ("time", Slash::Time),
            ("log", Slash::Log(None)),
            ("log my file.log", Slash::Log(Some("my file.log".into()))),
            ("send-file fw.bin", Slash::SendFile("fw.bin".into())),
            ("run boot.txt", Slash::Run("boot.txt".into())),
            ("raw", Slash::Raw),
            ("stop", Slash::Stop),
            ("echo", Slash::Echo),
            ("dtr", Slash::Dtr),
            ("rts", Slash::Rts),
            ("break", Slash::Break),
            ("reset", Slash::Reset),
            ("clear", Slash::Clear),
            ("HELP", Slash::Help),
            ("quit", Slash::Quit),
            ("exit", Slash::Quit),
            ("close", Slash::Close),
            ("groups", Slash::Groups(None)),
            (
                "groups my cmds.json",
                Slash::Groups(Some("my cmds.json".into())),
            ),
            ("group boot", Slash::Group("boot".into())),
            ("cmd 2", Slash::Cmd("2".into())),
            ("cmd", Slash::Invalid("cmd".into())),
            ("plot", Slash::Plot(None)),
            ("plot t=(\\d+)", Slash::Plot(Some("t=(\\d+)".into()))),
            ("plot a  b=(\\d) ", Slash::Plot(Some("a  b=(\\d)".into()))),
            (
                "open /dev/ttyUSB1",
                Slash::Open {
                    port: "/dev/ttyUSB1".into(),
                    baud: None,
                },
            ),
            (
                "open COM3 9600",
                Slash::Open {
                    port: "COM3".into(),
                    baud: Some(9600),
                },
            ),
            ("open COM3 fast", Slash::Invalid("open COM3 fast".into())),
            ("open a b c", Slash::Invalid("open a b c".into())),
            ("baud", Slash::Invalid("baud".into())),
            ("baud fast", Slash::Invalid("baud fast".into())),
            ("baud 0", Slash::Invalid("baud 0".into())),
            ("hex now", Slash::Invalid("hex now".into())),
            ("send-file", Slash::Invalid("send-file".into())),
            ("nope", Slash::Invalid("nope".into())),
            ("", Slash::Invalid(String::new())),
        ];
        for (text, want) in cases {
            assert_eq!(Slash::parse(text), want, "{text:?}");
        }
        // Every listed command parses; required arguments are required.
        for (name, arg) in SLASH_COMMANDS {
            let bare = matches!(Slash::parse(name), Slash::Invalid(_));
            assert_eq!(bare, arg.starts_with('<'), "{name}");
        }
    }

    #[test]
    fn tab_completes_commands() {
        let mut keys = line();
        typing(&mut keys, "/ba");
        keys.key(press(KeyCode::Tab));
        assert_eq!(keys.editor.text(), "/baud ");
        keys.editor.clear();
        typing(&mut keys, "/r");
        keys.key(press(KeyCode::Tab));
        assert_eq!(
            keys.editor.text(),
            "/r",
            "raw, reset, rts, run share only r"
        );
        typing(&mut keys, "e");
        keys.key(press(KeyCode::Tab));
        assert_eq!(keys.editor.text(), "/reset ");
        keys.editor.clear();
        typing(&mut keys, "AT");
        keys.key(press(KeyCode::Tab));
        assert_eq!(keys.editor.text(), "AT", "only commands complete");
        keys.editor.clear();
        typing(&mut keys, "/zz");
        keys.key(press(KeyCode::Tab));
        assert_eq!(keys.editor.text(), "/zz");
        keys.editor.clear();
        typing(&mut keys, "/log x");
        keys.key(press(KeyCode::Tab));
        assert_eq!(keys.editor.text(), "/log x");
    }

    #[test]
    fn editing_keys_work_like_a_shell() {
        let mut keys = line();
        typing(&mut keys, "AT+X");
        keys.key(press(KeyCode::Backspace));
        keys.key(press(KeyCode::Left));
        keys.key(press(KeyCode::Delete));
        keys.key(press(KeyCode::Home));
        keys.key(press(KeyCode::Right));
        keys.key(press(KeyCode::End));
        assert_eq!(keys.editor.text(), "AT");
        keys.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        typing(&mut keys, "two");
        assert_eq!(keys.editor.text(), "AT\ntwo");
        keys.key(ctrl('w'));
        keys.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        typing(&mut keys, "x");
        keys.key(ctrl('u'));
        assert_eq!(keys.editor.text(), "AT\n\n", "only the current line");
        keys.key(ctrl('a'));
        keys.key(press(KeyCode::Esc));
        keys.key(ctrl('e'));
        assert_eq!(keys.editor.cursor(), 4);
        keys.key(press(KeyCode::Esc));
        assert!(keys.editor.is_empty());
        assert_eq!(keys.key(ctrl('l')), [Action::Local(Local::Clear)]);
        assert!(
            keys.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT))
                .is_empty()
        );
        assert!(keys.key(press(KeyCode::F(5))).is_empty());
    }

    #[test]
    fn ctrl_c_and_ctrl_d_clear_or_reach_the_device() {
        let mut keys = line();
        typing(&mut keys, "oops");
        assert_eq!(keys.key(ctrl('c')), [Action::Edited]);
        assert!(keys.editor.is_empty());
        assert_eq!(
            keys.key(ctrl('c')),
            [Action::Send(vec![0x03])],
            "empty: interrupt the device"
        );
        assert_eq!(keys.key(ctrl('d')), [Action::Send(vec![0x04])]);
        typing(&mut keys, "ab");
        keys.key(press(KeyCode::Left));
        keys.key(ctrl('d'));
        assert_eq!(keys.editor.text(), "a");
    }

    #[test]
    fn history_and_reverse_search() {
        let mut keys = line();
        for line in ["AT+CWLAP", "led on", "AT+GMR"] {
            typing(&mut keys, line);
            keys.key(press(KeyCode::Enter));
        }
        keys.key(press(KeyCode::Up));
        assert_eq!(keys.editor.text(), "AT+GMR");
        keys.key(press(KeyCode::Down));
        assert!(keys.editor.is_empty());
        assert!(keys.key(press(KeyCode::Down)).is_empty(), "nothing below");

        keys.key(ctrl('r'));
        typing(&mut keys, "AT");
        assert_eq!(keys.search.as_ref().unwrap().found, Some(2));
        keys.key(ctrl('r'));
        assert_eq!(keys.search.as_ref().unwrap().found, Some(0), "older");
        keys.key(press(KeyCode::Backspace));
        assert_eq!(
            keys.search.as_ref().unwrap().found,
            Some(2),
            "A matches the newest again"
        );
        keys.key(press(KeyCode::Enter));
        assert_eq!(keys.editor.text(), "AT+GMR");
        assert!(keys.search.is_none());
        keys.key(ctrl('r'));
        assert!(keys.key(press(KeyCode::F(1))).is_empty());
        keys.key(press(KeyCode::Esc));
        assert!(keys.search.is_none());
        keys.key(ctrl('r'));
        keys.key(ctrl('g'));
        assert!(keys.search.is_none());
    }

    #[test]
    fn raw_mode_sends_every_key() {
        let mut keys = Keys::new(Mode::Raw, 'a');
        assert_eq!(
            keys.key(press(KeyCode::Char('é'))),
            [Action::Send("é".as_bytes().to_vec())]
        );
        assert_eq!(keys.key(ctrl('c')), [Action::Send(vec![0x03])]);
        assert_eq!(
            keys.key(press(KeyCode::Up)),
            [Action::Send(b"\x1b[A".to_vec())]
        );
        assert_eq!(keys.paste("ls\n"), [Action::Send(b"ls\n".to_vec())]);
        assert!(keys.key(press(KeyCode::CapsLock)).is_empty());
        let mut released = press(KeyCode::Char('x'));
        released.kind = KeyEventKind::Release;
        assert!(keys.key(released).is_empty());
        let mut line = line();
        assert_eq!(line.paste("a\nb"), [Action::Edited]);
        assert_eq!(line.editor.text(), "a\nb");
    }

    #[test]
    fn raw_bytes_match_an_xterm() {
        let cases: &[(KeyEvent, &[u8])] = &[
            (press(KeyCode::Enter), b"\r"),
            (press(KeyCode::Tab), b"\t"),
            (press(KeyCode::BackTab), b"\x1b[Z"),
            (press(KeyCode::Backspace), b"\x7f"),
            (press(KeyCode::Esc), b"\x1b"),
            (press(KeyCode::Down), b"\x1b[B"),
            (press(KeyCode::Right), b"\x1b[C"),
            (press(KeyCode::Left), b"\x1b[D"),
            (press(KeyCode::Home), b"\x1b[H"),
            (press(KeyCode::End), b"\x1b[F"),
            (press(KeyCode::Insert), b"\x1b[2~"),
            (press(KeyCode::Delete), b"\x1b[3~"),
            (press(KeyCode::PageUp), b"\x1b[5~"),
            (press(KeyCode::PageDown), b"\x1b[6~"),
            (press(KeyCode::F(1)), b"\x1bOP"),
            (press(KeyCode::F(4)), b"\x1bOS"),
            (press(KeyCode::F(5)), b"\x1b[15~"),
            (press(KeyCode::F(12)), b"\x1b[24~"),
            (ctrl(' '), b"\x00"),
            (ctrl('Z'), b"\x1a"),
            (
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
                b"\x1bb",
            ),
        ];
        for (key, want) in cases {
            assert_eq!(raw_bytes(*key).as_deref(), Some(*want), "{key:?}");
        }
        assert_eq!(raw_bytes(press(KeyCode::F(13))), None);
    }
}
