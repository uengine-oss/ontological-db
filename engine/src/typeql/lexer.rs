//! TypeQL tokeniser — spec 010 T001.
//!
//! Two things make this materially different from the Cypher lexer next door:
//!
//! 1. **Labels contain hyphens.** `isbn-13`, `order-line` and `start-timestamp`
//!    are single names, not subtractions. A `-` is absorbed into an identifier
//!    only when it sits directly between two identifier characters, so
//!    `1 - $d` and `$a-$b` still lex as arithmetic.
//! 2. **Datetimes are unquoted.** `2023-12-02T00:00:00` is a literal, and its
//!    first character is a digit — so a digit does not imply a number.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Type labels, role names and every keyword. The parser decides which.
    Ident(String),
    /// `$name`
    Var(String),
    Str(String),
    Int(i64),
    Float(f64),
    /// Raw text of a date or datetime literal, kept verbatim for the cast.
    DateTime(String),
    Sym(&'static str),
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "{s}"),
            Tok::Var(s) => write!(f, "${s}"),
            Tok::Str(s) => write!(f, "\"{s}\""),
            Tok::Int(i) => write!(f, "{i}"),
            Tok::Float(x) => write!(f, "{x}"),
            Tok::DateTime(s) => write!(f, "{s}"),
            Tok::Sym(s) => write!(f, "{s}"),
            Tok::Eof => write!(f, "end of input"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

/// Multi-character symbols, longest first so `<=` never lexes as `<` then `=`.
const SYMBOLS: &[&str] = &[
    "..", "==", "!=", "<=", ">=", "->", "=", "<", ">", "+", "-", "*", "/", "%", ";", ",", "(", ")",
    "{", "}", "[", "]", ":", ".", "@", "?", "|",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    macro_rules! bump {
        ($n:expr) => {{
            for _ in 0..$n {
                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }
        }};
    }

    while i < chars.len() {
        let c = chars[i];

        if c.is_whitespace() {
            bump!(1);
            continue;
        }

        // Comment to end of line.
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                bump!(1);
            }
            continue;
        }

        let (sl, sc) = (line, col);

        // Variable.
        if c == '$' {
            bump!(1);
            let start = i;
            while i < chars.len()
                && (is_ident_char(chars[i])
                    || (chars[i] == '-'
                        && i + 1 < chars.len()
                        && is_ident_char(chars[i + 1])
                        && i > start))
            {
                bump!(1);
            }
            if i == start {
                return Err(format!("line {sl}:{sc}: '$' must be followed by a variable name"));
            }
            let name: String = chars[start..i].iter().collect();
            out.push(Token { tok: Tok::Var(name), line: sl, col: sc });
            continue;
        }

        // String literal.
        if c == '"' || c == '\'' {
            let quote = c;
            bump!(1);
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err(format!("line {sl}:{sc}: unterminated string literal"));
                }
                let ch = chars[i];
                if ch == '\\' && i + 1 < chars.len() {
                    let esc = chars[i + 1];
                    s.push(match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                    bump!(2);
                    continue;
                }
                if ch == quote {
                    bump!(1);
                    break;
                }
                s.push(ch);
                bump!(1);
            }
            out.push(Token { tok: Tok::Str(s), line: sl, col: sc });
            continue;
        }

        // Number, date or datetime — all begin with a digit.
        if c.is_ascii_digit() {
            if let Some(len) = scan_datetime(&chars, i) {
                let text: String = chars[i..i + len].iter().collect();
                bump!(len);
                out.push(Token { tok: Tok::DateTime(text), line: sl, col: sc });
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                bump!(1);
            }
            // A `.` is part of the number only when it is not the `..` of a range.
            let mut is_float = false;
            if i < chars.len()
                && chars[i] == '.'
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
            {
                is_float = true;
                bump!(1);
                while i < chars.len() && chars[i].is_ascii_digit() {
                    bump!(1);
                }
            }
            let text: String = chars[start..i].iter().collect();
            let tok = if is_float {
                Tok::Float(text.parse::<f64>().map_err(|_| format!("bad number '{text}'"))?)
            } else {
                match text.parse::<i64>() {
                    Ok(v) => Tok::Int(v),
                    Err(_) => Tok::Float(
                        text.parse::<f64>().map_err(|_| format!("bad number '{text}'"))?,
                    ),
                }
            };
            out.push(Token { tok, line: sl, col: sc });
            continue;
        }

        // Identifier / keyword / label.
        if is_ident_start(c) {
            let start = i;
            bump!(1);
            loop {
                if i < chars.len() && is_ident_char(chars[i]) {
                    bump!(1);
                    continue;
                }
                // Hyphen glues only when flanked by identifier characters.
                if i + 1 < chars.len() && chars[i] == '-' && is_ident_char(chars[i + 1]) {
                    bump!(2);
                    continue;
                }
                break;
            }
            // `isa!`, `sub!` — the exact-type variants.
            if i < chars.len() && chars[i] == '!' {
                bump!(1);
            }
            let name: String = chars[start..i].iter().collect();
            out.push(Token { tok: Tok::Ident(name), line: sl, col: sc });
            continue;
        }

        // Symbols.
        let mut matched = None;
        for sym in SYMBOLS {
            let n = sym.chars().count();
            if i + n <= chars.len() && chars[i..i + n].iter().collect::<String>() == **sym {
                matched = Some((*sym, n));
                break;
            }
        }
        match matched {
            Some((sym, n)) => {
                bump!(n);
                out.push(Token { tok: Tok::Sym(sym), line: sl, col: sc });
            }
            None => {
                return Err(format!("line {sl}:{sc}: unexpected character '{c}'"));
            }
        }
    }

    out.push(Token { tok: Tok::Eof, line, col });
    Ok(out)
}

/// Length of a `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS[.fff]]` literal at `i`, if any.
fn scan_datetime(chars: &[char], i: usize) -> Option<usize> {
    let digits = |at: usize, n: usize| -> bool {
        at + n <= chars.len() && (0..n).all(|k| chars[at + k].is_ascii_digit())
    };
    if !(digits(i, 4) && i + 4 < chars.len() && chars[i + 4] == '-' && digits(i + 5, 2)) {
        return None;
    }
    if !(i + 7 < chars.len() && chars[i + 7] == '-' && digits(i + 8, 2)) {
        return None;
    }
    let mut len = 10;
    if i + len < chars.len() && chars[i + len] == 'T' && digits(i + len + 1, 2) {
        len += 3;
        if i + len < chars.len() && chars[i + len] == ':' && digits(i + len + 1, 2) {
            len += 3;
            if i + len < chars.len() && chars[i + len] == ':' && digits(i + len + 1, 2) {
                len += 3;
                if i + len < chars.len() && chars[i + len] == '.' {
                    let mut k = len + 1;
                    while i + k < chars.len() && chars[i + k].is_ascii_digit() {
                        k += 1;
                    }
                    if k > len + 1 {
                        len = k;
                    }
                }
            }
        }
    }
    Some(len)
}

#[cfg(test)]
mod unit {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn hyphenated_labels_are_one_token() {
        assert_eq!(
            kinds("isbn-13"),
            vec![Tok::Ident("isbn-13".into()), Tok::Eof]
        );
        assert_eq!(
            kinds("$start-timestamp"),
            vec![Tok::Var("start-timestamp".into()), Tok::Eof]
        );
    }

    #[test]
    fn subtraction_still_lexes_as_arithmetic() {
        assert_eq!(
            kinds("1 - $d"),
            vec![Tok::Int(1), Tok::Sym("-"), Tok::Var("d".into()), Tok::Eof]
        );
        assert_eq!(
            kinds("$a-$b"),
            vec![Tok::Var("a".into()), Tok::Sym("-"), Tok::Var("b".into()), Tok::Eof]
        );
    }

    #[test]
    fn datetimes_are_literals_not_subtractions() {
        assert_eq!(
            kinds("2023-12-02T00:00:00"),
            vec![Tok::DateTime("2023-12-02T00:00:00".into()), Tok::Eof]
        );
        assert_eq!(kinds("2023-12-02"), vec![Tok::DateTime("2023-12-02".into()), Tok::Eof]);
    }

    #[test]
    fn ranges_do_not_swallow_the_dots() {
        assert_eq!(
            kinds("@card(0..2)"),
            vec![
                Tok::Sym("@"),
                Tok::Ident("card".into()),
                Tok::Sym("("),
                Tok::Int(0),
                Tok::Sym(".."),
                Tok::Int(2),
                Tok::Sym(")"),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn exact_type_variants_keep_their_bang() {
        assert_eq!(kinds("isa!"), vec![Tok::Ident("isa!".into()), Tok::Eof]);
    }

    #[test]
    fn comments_and_strings() {
        assert_eq!(
            kinds("# note\n\"a b\" 3.5"),
            vec![Tok::Str("a b".into()), Tok::Float(3.5), Tok::Eof]
        );
    }
}
