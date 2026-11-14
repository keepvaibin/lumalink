//! The one style of LumaScript (`lumalink format` and the editor's formatting). It sets where each line
//! starts and nothing inside a line: four spaces for each bracket open around the line, and four more for a line that
//! goes on with the statement of the line before (after an operator, `and`, `or` or `not`). A line that starts
//! with a closing bracket stands where the line of its opening bracket stands. Each line loses the spaces at its end,
//! runs of blank lines become one, the file has none at its start or end, ends in one line end, and every line end is
//! LF. Strings and comments are never changed inside; a line that starts inside a block comment keeps its own
//! leading spaces. What the formatter writes reads to the same tokens as what it read, and formatting it again
//! changes nothing.

use crate::lex::{is_word, CONTINUES, OPERATORS};

const INDENT: usize = 4;

/// The last token of a line as far as the next line cares: whether the statement goes on past the line end.
#[derive(Clone, Copy, PartialEq)]
enum Last {
    /// Nothing yet: the line holds only a comment, or the file starts.
    Nothing,
    /// The statement goes on: an operator of [`CONTINUES`] but a bracket and `,`, or `and`, `or`, `not`.
    GoesOn,
    /// `.`, `::` or `?.`: the word after it is a name, never `and`/`or`/`not`. The statement goes on too.
    Member,
    /// Anything else: a name, a number, a string, a closing or an opening bracket (the bracket indents by itself),
    /// `,` (a list's items stand one under another).
    Ends,
}

/// What is open at a point of the file: its brackets, each with the indent of the line it opened on, and how deep the
/// block comments go.
#[derive(Default)]
struct State {
    open: Vec<(char, usize)>,
    comment: u32,
}

impl State {
    /// Closes back to `c`'s own opener, as the lexer's recovery does (one with no opener closes nothing); gives where
    /// that opener stood in `open` and the indent it opened at.
    fn close(&mut self, c: char) -> Option<(usize, usize)> {
        let opener = opener_of(c);
        let k = self.open.iter().rposition(|&(o, _)| o == opener)?;
        let at = self.open[k].1;
        self.open.truncate(k);
        Some((k, at))
    }
}

fn opener_of(c: char) -> char {
    match c {
        ')' => '(',
        ']' => '[',
        _ => '{',
    }
}

/// The text in the one style. Formatting what this gives changes nothing.
pub fn format(src: &str) -> String {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src).replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(src.len() + src.len() / 8 + 1);
    let mut state = State::default();
    let mut last = Last::Nothing;
    let (mut started, mut blank) = (false, false);
    for raw in src.split('\n') {
        let line = raw.trim_end();
        let in_comment = state.comment > 0;
        let body = if in_comment { line } else { line.trim_start() };
        if body.trim_start().is_empty() {
            blank = started;
            continue;
        }
        if blank {
            out.push('\n');
            blank = false;
        }
        started = true;
        let chars: Vec<char> = body.chars().collect();
        let indent = if in_comment {
            // The comment's own text, kept as written; brackets after its end open at its width.
            out.push_str(body);
            body.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { INDENT } else { 1 }).sum()
        } else {
            let indent = indent_of(&chars, &state, last);
            out.extend(std::iter::repeat_n(' ', indent));
            out.push_str(body);
            indent
        };
        out.push('\n');
        if let Some(l) = scan(&chars, &mut state, indent, last) {
            last = l;
        }
    }
    out
}

/// Where a code line starts: under the line of the bracket it closes first; else four in from the line of the
/// innermost open bracket, four more when it goes on with the statement before (where a line end could end one).
fn indent_of(chars: &[char], state: &State, last: Last) -> usize {
    let first = chars[0];
    if matches!(first, ')' | ']' | '}') {
        let opener = opener_of(first);
        if let Some(&(_, at)) = state.open.iter().rev().find(|&&(o, _)| o == opener) {
            return at;
        }
    }
    let inner = state.open.last().copied();
    let mut indent = inner.map_or(0, |(_, at)| at + INDENT);
    // Inside ( and [ every line end goes on already; `else` stands with its `if`.
    let ends_statements = inner.is_none_or(|(o, _)| o == '{');
    if ends_statements && matches!(last, Last::GoesOn | Last::Member) && !starts_word(chars, "else") {
        indent += INDENT;
    }
    indent
}

fn starts_word(chars: &[char], word: &str) -> bool {
    let n = word.chars().count();
    chars.len() >= n && chars[..n].iter().copied().eq(word.chars()) && !chars.get(n).is_some_and(|&c| is_word(c))
}

/// Reads one line (its text from where it starts, at `indent`) into `state`; gives its last token, None when it has
/// none (a comment, or all of it inside one). A bracket opens at the line's indent; but once the line has closed one
/// that an earlier line opened, the brackets it opens after stand where that one did (`fn f(a: int,` / `b: int) {`
/// and `} else {`: the block's lines are four in from the statement's).
fn scan(chars: &[char], state: &mut State, indent: usize, before: Last) -> Option<Last> {
    let n = chars.len();
    let at = |k: usize| chars.get(k).copied();
    let mut last: Option<Last> = None;
    // The brackets of `open` below `earlier` are earlier lines'; `anchor` is where this line's next opener stands.
    let (mut earlier, mut anchor) = (state.open.len(), indent);
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if state.comment > 0 {
            if c == '/' && at(i + 1) == Some('*') {
                state.comment += 1;
                i += 2;
            } else if c == '*' && at(i + 1) == Some('/') {
                state.comment -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        match c {
            ' ' | '\t' => i += 1,
            '/' if at(i + 1) == Some('/') => break,
            '/' if at(i + 1) == Some('*') => {
                state.comment = 1;
                i += 2;
            }
            '"' => {
                i = string_end(chars, i + 1, Prefix::Plain);
                last = Some(Last::Ends);
            }
            '`' => {
                i = (i + 1..n).find(|&k| chars[k] == '`').map_or(n, |k| k + 1);
                last = Some(Last::Ends);
            }
            '(' | '[' | '{' => {
                state.open.push((c, anchor));
                last = Some(Last::Ends);
                i += 1;
            }
            ')' | ']' | '}' => {
                if let Some((k, at)) = state.close(c) {
                    if k < earlier {
                        (earlier, anchor) = (k, at);
                    }
                }
                last = Some(Last::Ends);
                i += 1;
            }
            _ if (c == 'r' || c == 'b') && at(i + 1) == Some('"') => {
                i = string_end(chars, i + 2, if c == 'r' { Prefix::Raw } else { Prefix::Bytes });
                last = Some(Last::Ends);
            }
            _ if is_word(c) => {
                let from = i;
                while i < n && is_word(chars[i]) {
                    i += 1;
                }
                let word: String = chars[from..i].iter().collect();
                // After `.` (this line's, or ending the line before) a word is a name.
                let member = last.map_or(before == Last::Member, |l| l == Last::Member);
                last = Some(if !member && CONTINUES.contains(&word.as_str()) { Last::GoesOn } else { Last::Ends });
            }
            _ => {
                // An operator: the lexer's, longest first.
                let rest: String = chars[i..n.min(i + 3)].iter().collect();
                match OPERATORS.iter().find(|op| rest.starts_with(**op)) {
                    Some(op) => {
                        i += op.chars().count();
                        last = Some(match *op {
                            "." | "::" | "?." => Last::Member,
                            "," => Last::Ends,
                            op if CONTINUES.contains(&op) => Last::GoesOn,
                            _ => Last::Ends,
                        });
                    }
                    None => {
                        // `#` of a colour, or a character the lexer refuses: it ends nothing more than a name does.
                        i += 1;
                        last = Some(Last::Ends);
                    }
                }
            }
        }
    }
    last
}

#[derive(Clone, Copy, PartialEq)]
enum Prefix {
    Plain,
    Raw,
    Bytes,
}

/// Where a string whose text starts at `j` ends (past its closing quote), read as the lexer reads it: on one line,
/// `\` escapes but in a raw string, `\u{...}`'s braces are the escape's, `{{` is a brace, `{` starts an
/// interpolation (in a plain string) that runs to its matching brace with any string inside it skipped whole. One the
/// line does not close runs to the line's end.
fn string_end(chars: &[char], mut j: usize, prefix: Prefix) -> usize {
    let n = chars.len();
    while j < n {
        let ch = chars[j];
        if ch == '"' {
            return j + 1;
        }
        if ch == '\\' && prefix != Prefix::Raw {
            if j + 1 >= n {
                return n;
            }
            let next = chars[j + 1];
            j += 2;
            if next == 'u' && chars.get(j) == Some(&'{') {
                if let Some(close) = (j..n.min(j + 10)).find(|&k| chars[k] == '}') {
                    j = close + 1;
                }
            }
            continue;
        }
        if ch == '{' && prefix == Prefix::Plain {
            if chars.get(j + 1) == Some(&'{') {
                j += 2;
                continue;
            }
            let (mut depth, mut k) = (1u32, j + 1);
            while k < n && depth > 0 {
                match chars[k] {
                    '"' => {
                        k += 1;
                        while k < n && chars[k] != '"' {
                            k += if chars[k] == '\\' && k + 1 < n { 2 } else { 1 };
                        }
                        if k >= n {
                            break;
                        }
                    }
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                k += 1;
            }
            if depth > 0 {
                return n;
            }
            j = k;
            continue;
        }
        j += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn indents_by_brackets_and_goes_on() {
        let src = "  var x = 1   \r\n\r\n\r\nfn f(a: int,\nb: int) {\n\tif a > b {\nlog \"{a} {{ ( \"\n  } else {\n    x = a +\n  b\n      }\n}\n\n";
        assert_eq!(
            format(src),
            "var x = 1\n\nfn f(a: int,\n    b: int) {\n    if a > b {\n        log \"{a} {{ ( \"\n    } else {\n        x = a +\n            b\n    }\n}\n"
        );
    }
}
