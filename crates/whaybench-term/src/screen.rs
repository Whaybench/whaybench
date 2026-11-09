//! Reading what Claude Code draws, and keeping its window titles from reaching the terminal.

const COMMANDS: [&str; 4] = ["exit", "quit", "background", "bg"];

/// The command on Claude Code's input line, when it is the only thing typed there. The line is read as drawn:
/// a prompt mark, then the command, then possibly the input box's right border.
pub fn typed_command(line: &str) -> Option<&'static str> {
    let t = line.trim_end();
    let t = t.strip_suffix(['│', '|']).unwrap_or(t).trim_end();
    let slash = t.rfind('/')?;
    let name = COMMANDS.iter().find(|c| **c == &t[slash + 1..])?;
    let before = t[..slash].trim_end();
    before.ends_with(['>', '❯', '›']).then_some(*name)
}

/// Removes the window title sequences (OSC 0, 1, 2) from what goes to the terminal: the tab's title is the
/// session's name, which this program sets. The sequence can arrive split over reads.
#[derive(Default)]
pub struct Titles { state: State, held: Vec<u8> }

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Text,
    Esc,
    Osc,
    Title,
    TitleEsc,
}

impl Titles {
    pub fn strip(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            match self.state {
                State::Text => {
                    if b == 0x1b { self.state = State::Esc; self.held = vec![b]; } else { out.push(b); }
                }
                State::Esc => {
                    self.held.push(b);
                    if b == b']' { self.state = State::Osc; } else { out.append(&mut self.held); self.state = State::Text; }
                }
                State::Osc => {
                    // The number: 0, 1 or 2 followed by ';' is a title. Anything else passes.
                    self.held.push(b);
                    let number = &self.held[2..];
                    if number.len() == 2 && matches!(number[0], b'0' | b'1' | b'2') && number[1] == b';' {
                        self.state = State::Title;
                        self.held.clear();
                    } else if number.len() >= 2 || !matches!(number[0], b'0' | b'1' | b'2') {
                        out.append(&mut self.held);
                        self.state = State::Text;
                    }
                }
                State::Title => {
                    if b == 0x07 { self.state = State::Text; } else if b == 0x1b { self.state = State::TitleEsc; }
                }
                State::TitleEsc => { self.state = State::Text; }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_command_alone_on_the_input_line() {
        assert_eq!(typed_command("> /exit"), Some("exit"));
        assert_eq!(typed_command("❯ /quit   "), Some("quit"));
        assert_eq!(typed_command("│ > /background          │"), Some("background"));
        assert_eq!(typed_command("› /bg |"), Some("bg"));
    }
    #[test]
    fn not_a_command_when_more_is_typed_or_no_prompt() {
        assert_eq!(typed_command("> /exit now"), None);
        assert_eq!(typed_command("> tell me about /exit"), None);
        assert_eq!(typed_command("/exit"), None);
        assert_eq!(typed_command("> /exits"), None);
        assert_eq!(typed_command("> /home/user/bg"), None);
        assert_eq!(typed_command(""), None);
    }
    #[test]
    fn titles_are_removed_and_the_rest_stays() {
        let mut t = Titles::default();
        assert_eq!(t.strip(b"a\x1b]0;name\x07b"), b"ab");
        assert_eq!(t.strip(b"a\x1b]2;name\x1b\\b"), b"ab");
        assert_eq!(t.strip(b"\x1b[31mred\x1b[0m"), b"\x1b[31mred\x1b[0m");
        assert_eq!(t.strip(b"\x1b]8;;http://x\x07link"), b"\x1b]8;;http://x\x07link");
        assert_eq!(t.strip("é ü".as_bytes()), "é ü".as_bytes());
    }
    #[test]
    fn a_title_split_over_reads_is_still_removed() {
        let mut t = Titles::default();
        let mut out = t.strip(b"a\x1b");
        out.extend(t.strip(b"]0"));
        out.extend(t.strip(b";na"));
        out.extend(t.strip(b"me\x07b"));
        assert_eq!(out, b"ab");
    }
}
