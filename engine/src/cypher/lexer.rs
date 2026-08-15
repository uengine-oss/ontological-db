//! Cypher lexer — spec 003 FR-001.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// Back-quoted identifier: `my label`
    QuotedIdent(String),
    Keyword(String),
    Int(i64),
    Float(f64),
    Str(String),
    Param(String),
    Punct(String),
    Eof,
}

pub const KEYWORDS: &[&str] = &[
    "match", "optional", "where", "return", "with", "create", "merge", "set", "remove", "delete",
    "detach", "order", "by", "skip", "limit", "distinct", "as", "and", "or", "not", "xor", "in",
    "is", "null", "true", "false", "unwind", "union", "all", "asc", "desc", "starts", "ends",
    "contains", "case", "when", "then", "else", "end", "exists", "count", "on", "call", "yield",
];

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0 }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.peek_byte() {
                Some(c) if c.is_ascii_whitespace() => self.pos += 1,
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(c) = self.peek_byte() {
                        self.pos += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                Some(b'/') if self.src.get(self.pos + 1) == Some(&b'*') => {
                    self.pos += 2;
                    while self.pos < self.src.len() {
                        if self.src[self.pos] == b'*' && self.src.get(self.pos + 1) == Some(&b'/') {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.peek_byte() else {
                out.push(Token { tok: Tok::Eof, pos: start });
                return Ok(out);
            };

            let tok = match c {
                b'`' => {
                    self.pos += 1;
                    let s = self.take_until(b'`')?;
                    Tok::QuotedIdent(s)
                }
                b'\'' | b'"' => {
                    self.pos += 1;
                    Tok::Str(self.take_string(c)?)
                }
                b'$' => {
                    self.pos += 1;
                    let name = self.take_word();
                    if name.is_empty() {
                        return Err(format!("expected a parameter name after '$' at offset {start}"));
                    }
                    Tok::Param(name)
                }
                c if c.is_ascii_digit() => self.take_number()?,
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    let w = self.take_word();
                    let lw = w.to_ascii_lowercase();
                    if KEYWORDS.contains(&lw.as_str()) {
                        Tok::Keyword(lw)
                    } else {
                        Tok::Ident(w)
                    }
                }
                _ => self.take_punct()?,
            };
            out.push(Token { tok, pos: start });
        }
    }

    fn take_word(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek_byte() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }

    fn take_until(&mut self, end: u8) -> Result<String, String> {
        let start = self.pos;
        while let Some(c) = self.peek_byte() {
            if c == end {
                let s = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                self.pos += 1;
                return Ok(s);
            }
            self.pos += 1;
        }
        Err(format!("unterminated quoted identifier starting at offset {start}"))
    }

    fn take_string(&mut self, quote: u8) -> Result<String, String> {
        let mut s = String::new();
        while let Some(c) = self.peek_byte() {
            self.pos += 1;
            match c {
                b'\\' => {
                    let Some(esc) = self.peek_byte() else { break };
                    self.pos += 1;
                    s.push(match esc {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'0' => '\0',
                        other => other as char,
                    });
                }
                c if c == quote => return Ok(s),
                other => {
                    // Preserve multi-byte UTF-8 sequences verbatim.
                    if other < 0x80 {
                        s.push(other as char);
                    } else {
                        let start = self.pos - 1;
                        let len = utf8_len(other);
                        let end = (start + len).min(self.src.len());
                        s.push_str(&String::from_utf8_lossy(&self.src[start..end]));
                        self.pos = end;
                    }
                }
            }
        }
        Err("unterminated string literal".into())
    }

    fn take_number(&mut self) -> Result<Tok, String> {
        let start = self.pos;
        let mut is_float = false;
        while let Some(c) = self.peek_byte() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else if c == b'.' && !is_float && self.src.get(self.pos + 1).is_some_and(|d| d.is_ascii_digit()) {
                is_float = true;
                self.pos += 1;
            } else if (c == b'e' || c == b'E') && self.pos > start {
                is_float = true;
                self.pos += 1;
                if matches!(self.peek_byte(), Some(b'+') | Some(b'-')) {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let text = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        if is_float {
            text.parse::<f64>().map(Tok::Float).map_err(|e| e.to_string())
        } else {
            text.parse::<i64>().map(Tok::Int).map_err(|e| e.to_string())
        }
    }

    fn take_punct(&mut self) -> Result<Tok, String> {
        const THREE: &[&str] = &["<->"];
        const TWO: &[&str] = &["<=", ">=", "<>", "!=", "->", "<-", "..", "=~", "+=", "||"];
        let rest = &self.src[self.pos..];
        for p in THREE {
            if rest.starts_with(p.as_bytes()) {
                self.pos += 3;
                return Ok(Tok::Punct((*p).into()));
            }
        }
        for p in TWO {
            if rest.starts_with(p.as_bytes()) {
                self.pos += 2;
                return Ok(Tok::Punct((*p).into()));
            }
        }
        let c = rest[0];
        if b"()[]{}<>=+-*/%^.,:;|".contains(&c) {
            self.pos += 1;
            return Ok(Tok::Punct((c as char).to_string()));
        }
        Err(format!("unexpected character '{}' at offset {}", c as char, self.pos))
    }
}

fn utf8_len(b: u8) -> usize {
    if b >= 0xF0 {
        4
    } else if b >= 0xE0 {
        3
    } else if b >= 0xC0 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        Lexer::new(s).tokenize().unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn basic_pattern() {
        let t = toks("MATCH (a:Person)-[:KNOWS*1..3]->(b) RETURN b.name");
        assert_eq!(t[0], Tok::Keyword("match".into()));
        assert!(t.contains(&Tok::Punct("->".into())));
        assert!(t.contains(&Tok::Punct("..".into())));
        assert!(t.contains(&Tok::Ident("KNOWS".into())));
    }

    #[test]
    fn strings_and_params() {
        let t = toks(r#"WHERE a.name = $name OR a.x = 'it\'s' "#);
        assert!(t.contains(&Tok::Param("name".into())));
        assert!(t.contains(&Tok::Str("it's".into())));
    }

    #[test]
    fn unicode_strings_survive() {
        let t = toks("RETURN '한글 테스트'");
        assert!(t.contains(&Tok::Str("한글 테스트".into())));
    }

    #[test]
    fn numbers() {
        let t = toks("RETURN 42, 3.5, 1e3");
        assert!(t.contains(&Tok::Int(42)));
        assert!(t.contains(&Tok::Float(3.5)));
    }
}
