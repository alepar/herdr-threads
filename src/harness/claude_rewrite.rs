//! Pure shell-line rewrite for Claude subagent Bash calls (spec §1, Claude row).
//!
//! A Claude subagent's PreToolUse payload carries `agent_id`; the hook uses this module to insert
//! `--agent-id=<id>` right after the command word of every simple command that runs
//! `herdr-threads` or `ht`, so the CLI can attribute the call to the subagent. The lexer is
//! deliberately small and conservative: a line it cannot parse, and any shape it does not
//! recognize (`eval`, `bash -c`, scripts, `$(...)`, a quoted or path-qualified command word), is
//! left alone and stays attributed to the parent session (TRUST-POLICY accepted limit).
use crate::protocol::ids::AgentId;

/// Insert `--agent-id=<id>` right after the command word of every simple command whose command
/// word is `herdr-threads` or `ht` (spec §1 Claude row). `None`: nothing to rewrite (including
/// any line the lexer cannot parse); the caller then leaves the tool input untouched.
pub fn rewrite_command(line: &str, agent: &AgentId) -> Option<String> {
    let commands = lex(line)?;
    let mut offsets = Vec::new();
    for words in &commands {
        let Some(at) = command_word(line, words) else {
            continue;
        };
        if words
            .get(at + 1)
            .is_some_and(|next| line[next.start..next.end].starts_with("--agent-id"))
        {
            continue;
        }
        offsets.push(words[at].end);
    }
    if offsets.is_empty() {
        return None;
    }
    let insert = format!(" --agent-id={}", quote(agent.as_str()));
    let mut out = String::with_capacity(line.len() + offsets.len() * insert.len());
    let mut last = 0;
    for offset in offsets {
        out.push_str(&line[last..offset]);
        out.push_str(&insert);
        last = offset;
    }
    out.push_str(&line[last..]);
    Some(out)
}

fn quote(id: &str) -> String {
    if id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'@' | b'+' | b'-'))
    {
        id.to_owned()
    } else {
        format!("'{}'", id.replace('\'', "'\\''"))
    }
}

/// One shell word as a byte range of the line.
#[derive(Clone, Copy)]
struct Word {
    start: usize,
    end: usize,
}

/// Split a line into simple commands, each a list of words. `None` for an unbalanced quote or
/// substitution, or a heredoc (whose body the lexer does not track).
fn lex(line: &str) -> Option<Vec<Vec<Word>>> {
    let bytes = line.as_bytes();
    let mut commands = vec![Vec::new()];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' | b';' | b'|' | b'&' => {
                i += 1;
                if matches!(bytes[i - 1], b'|' | b'&') && bytes.get(i) == Some(&bytes[i - 1]) {
                    i += 1;
                }
                commands.push(Vec::new());
            }
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {
                let start = i;
                i = word_end(bytes, i)?;
                commands.last_mut()?.push(Word { start, end: i });
            }
        }
    }
    Some(commands)
}

/// The index just past the word starting at `i`.
fn word_end(bytes: &[u8], mut i: usize) -> Option<usize> {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' => break,
            b'<' if bytes.get(i + 1) == Some(&b'<') => return None,
            b'\\' => {
                bytes.get(i + 1)?;
                i += 2;
            }
            b'\'' => {
                i += 1;
                while *bytes.get(i)? != b'\'' {
                    i += 1;
                }
                i += 1;
            }
            b'"' => i = double_quoted(bytes, i + 1)?,
            b'`' => i = backticked(bytes, i + 1)?,
            b'$' if bytes.get(i + 1) == Some(&b'(') => i = parenthesized(bytes, i + 2)?,
            _ => i += 1,
        }
    }
    Some(i)
}

/// After the opening `"`: the index past the closing one.
fn double_quoted(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        match *bytes.get(i)? {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            b'`' => i = backticked(bytes, i + 1)?,
            b'$' if bytes.get(i + 1) == Some(&b'(') => i = parenthesized(bytes, i + 2)?,
            _ => i += 1,
        }
    }
}

/// After the opening backtick: the index past the closing one.
fn backticked(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        match *bytes.get(i)? {
            b'`' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
}

/// After `$(`: the index past the matching `)`, counting nested parentheses and skipping quotes.
fn parenthesized(bytes: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 1usize;
    while depth > 0 {
        match *bytes.get(i)? {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            b'\\' => i += 2,
            b'\'' => {
                i += 1;
                while *bytes.get(i)? != b'\'' {
                    i += 1;
                }
                i += 1;
            }
            b'"' => i = double_quoted(bytes, i + 1)?,
            b'`' => i = backticked(bytes, i + 1)?,
            _ => i += 1,
        }
    }
    Some(i)
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The index of the `herdr-threads`/`ht` command word of one simple command, after leading
/// assignments and the wrappers `env`, `command`, `nice`, `timeout`, `time` and `nohup`.
fn command_word(line: &str, words: &[Word]) -> Option<usize> {
    let raw = |i: usize| words.get(i).map(|w| &line[w.start..w.end]);
    let mut i = 0;
    loop {
        let word = raw(i)?;
        if is_assignment(word) {
            i += 1;
            continue;
        }
        match word {
            "ht" | "herdr-threads" => return Some(i),
            "env" => {
                i += 1;
                while let Some(arg) = raw(i) {
                    match arg {
                        "-i" | "--" => i += 1,
                        "-u" => i += 2,
                        _ if is_assignment(arg) => i += 1,
                        _ => break,
                    }
                }
            }
            "command" => {
                i += 1;
                while let Some(arg) = raw(i) {
                    match arg {
                        "-p" => i += 1,
                        "-v" | "-V" => return None,
                        _ => break,
                    }
                }
            }
            "nice" => {
                i += 1;
                while let Some(arg) = raw(i) {
                    if arg == "-n" {
                        i += 2;
                    } else if arg.starts_with("--adjustment=")
                        || (arg.len() > 1
                            && arg.starts_with('-')
                            && arg[1..].bytes().all(|b| b.is_ascii_digit()))
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            "timeout" => {
                i += 1;
                while let Some(arg) = raw(i) {
                    match arg {
                        "--preserve-status" | "--foreground" => i += 1,
                        "-s" | "-k" | "--signal" | "--kill-after" => i += 2,
                        _ if arg.starts_with("--signal=") || arg.starts_with("--kill-after=") => {
                            i += 1;
                        }
                        _ => break,
                    }
                }
                i += 1; // the DURATION word
            }
            "time" => {
                i += 1;
                if raw(i) == Some("-p") {
                    i += 1;
                }
            }
            "nohup" => i += 1,
            _ => return None,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/harness/claude_rewrite.rs"]
mod tests;
