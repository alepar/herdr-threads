//! A styled terminal screen: the text of a Herdr `agent read --format ansi`
//! split into plain lines and per-character SGR styles, so a composer reader
//! can tell dim placeholder or suggestion text from typed text.
//!
//! Only SGR (`ESC [ … m`) sequences are understood. Any other escape or
//! control character, or an SGR code not listed here, makes the screen
//! unparsable (`None`), and so does a screen without a single SGR sequence:
//! a reader that cannot be sure it sees the styles keeps the plain read.

/// The attributes in effect for one character. `dim` is SGR 2; every other
/// attribute and any foreground, background or underline colour is tracked
/// separately so its own reset (22, 23, 39, ...) clears only it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    dim: bool,
    bold: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    inverse: bool,
    hidden: bool,
    strike: bool,
    overline: bool,
    fg: bool,
    bg: bool,
    underline_colour: bool,
}

impl Style {
    /// No attribute and no colour: the terminal's default rendition.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// SGR 2 and nothing else.
    pub fn is_dim_only(&self) -> bool {
        *self
            == Self {
                dim: true,
                ..Self::default()
            }
    }

    pub fn is_dim(&self) -> bool {
        self.dim
    }

    /// Applies one SGR parameter list; `None` on a code this reader does not know.
    fn apply(&mut self, params: &str) -> Option<()> {
        // `38:2:r:g:b` (colon sub-parameters) is one parameter.
        let mut codes = params.split(';').map(|param| {
            let head = param.split(':').next().unwrap_or_default();
            let sub = param.contains(':');
            (
                if head.is_empty() {
                    Some(0)
                } else {
                    head.parse::<u16>().ok()
                },
                sub,
            )
        });
        while let Some((code, sub)) = codes.next() {
            match code? {
                0 => *self = Self::default(),
                1 => self.bold = true,
                2 => self.dim = true,
                3 => self.italic = true,
                4 | 21 => self.underline = true,
                5 | 6 => self.blink = true,
                7 => self.inverse = true,
                8 => self.hidden = true,
                9 => self.strike = true,
                22 => (self.bold, self.dim) = (false, false),
                23 => self.italic = false,
                24 => self.underline = false,
                25 => self.blink = false,
                27 => self.inverse = false,
                28 => self.hidden = false,
                29 => self.strike = false,
                30..=37 | 90..=97 => self.fg = true,
                39 => self.fg = false,
                40..=47 | 100..=107 => self.bg = true,
                49 => self.bg = false,
                53 => self.overline = true,
                55 => self.overline = false,
                59 => self.underline_colour = false,
                extended @ (38 | 48 | 58) => {
                    // `;`-separated: `5;n` or `2;r;g;b` follow as their own parameters.
                    if !sub {
                        let arguments = match codes.next()?.0? {
                            5 => 1,
                            2 => 3,
                            _ => return None,
                        };
                        for _ in 0..arguments {
                            codes.next()?.0?;
                        }
                    }
                    match extended {
                        38 => self.fg = true,
                        48 => self.bg = true,
                        _ => self.underline_colour = true,
                    }
                }
                _ => return None,
            }
        }
        Some(())
    }
}

/// One screen row: its text and the style of each of its characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledLine {
    pub text: String,
    pub styles: Vec<Style>,
}

/// Parses an ANSI screen. Lines end in `\n`, optionally preceded by `\r`
/// (Herdr's ANSI reads use `\r\n`); the style carries across lines as a
/// terminal's would.
pub fn parse(screen: &str) -> Option<Vec<StyledLine>> {
    let mut lines = Vec::new();
    let mut line = StyledLine {
        text: String::new(),
        styles: Vec::new(),
    };
    let mut style = Style::default();
    let mut saw_sgr = false;
    let mut chars = screen.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                if chars.next()? != '[' {
                    return None;
                }
                let mut params = String::new();
                loop {
                    match chars.next()? {
                        'm' => break,
                        c @ ('0'..='9' | ';' | ':') => params.push(c),
                        _ => return None,
                    }
                }
                style.apply(&params)?;
                saw_sgr = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => lines.push(std::mem::replace(
                &mut line,
                StyledLine {
                    text: String::new(),
                    styles: Vec::new(),
                },
            )),
            c if c.is_control() => return None,
            c => {
                line.text.push(c);
                line.styles.push(style);
            }
        }
    }
    if !line.text.is_empty() {
        lines.push(line);
    }
    saw_sgr.then_some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(screen: &str) -> Vec<(char, Style)> {
        parse(screen)
            .unwrap()
            .into_iter()
            .flat_map(|line| line.text.chars().zip(line.styles).collect::<Vec<_>>())
            .collect()
    }

    #[test]
    fn dim_plain_and_coloured_runs_are_told_apart() {
        let read = styles("❯\u{a0}\x1b[0m\x1b[2mrun\x1b[0m x \x1b[38;2;1;2;3my\x1b[39mz\r\n");
        let text: String = read.iter().map(|(c, _)| c).collect();
        assert_eq!(text, "❯\u{a0}run x yz");
        let is = |index: usize| read[index].1;
        assert!(is(0).is_plain() && is(1).is_plain());
        assert!((2..5).all(|index| is(index).is_dim_only()));
        assert!((5..8).all(|index| is(index).is_plain()));
        assert!(!is(8).is_plain() && !is(8).is_dim());
        assert!(is(9).is_plain());
    }

    #[test]
    fn resets_clear_only_their_own_attribute() {
        let read = styles("\x1b[1;2ma\x1b[22mb\x1b[2;48;5;4mc\x1b[49md\x1b[mE\n");
        assert!(!read[0].1.is_dim_only() && read[0].1.is_dim());
        assert!(read[1].1.is_plain());
        assert!(read[2].1.is_dim() && !read[2].1.is_dim_only());
        assert!(read[3].1.is_dim_only());
        assert!(read[4].1.is_plain());
        let colon = styles("\x1b[2;38:2::1:2:3mx\x1b[39my\n");
        assert!(!colon[0].1.is_dim_only() && colon[1].1.is_dim_only());
    }

    #[test]
    fn style_carries_across_lines_and_rows_keep_their_text() {
        let lines = parse("\x1b[2mone\r\ntwo\x1b[0m\r\nthree").unwrap();
        let texts: Vec<_> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["one", "two", "three"]);
        assert!(lines[1].styles.iter().all(Style::is_dim_only));
        assert!(lines[2].styles.iter().all(Style::is_plain));
    }

    #[test]
    fn unknown_escapes_controls_and_unstyled_screens_are_unparsable() {
        for screen in [
            "plain text only\n",
            "\x1b[2mdim\x1b[0m \x1b[2Jcleared\n",
            "\x1b[2mdim\x1b]0;title\x07\n",
            "\x1b[2mdim\x1b[0m\tx\n",
            "\x1b[2mdim\x1b[0m\x1b[38;7mx\n",
            "\x1b[2mdim\x1b[0m\x1b[38;5mx\n",
            "\x1b[60mx\n",
            "\x1b[2",
        ] {
            assert_eq!(parse(screen), None, "{screen:?}");
        }
    }
}
