//! Minimal streaming lexer for Sail2 definition headers.
//!
//! The lexer is a pull-iterator over `&str`: it yields one [`Token`] at a
//! time and never buffers the whole file. The definition-level
//! [`crate::parser`] only needs identifiers, punctuation depth and line
//! structure; full expression lexing is a later-lane concern.

use crate::span::{FileId, Span, Spanned};

/// Token kinds needed for definition-level splitting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    /// `$name` pragma introducer (without the `$`).
    Pragma(String),
    Ident(String),
    /// Integer literal (kept raw).
    Int(String),
    /// String literal including quotes (kept raw).
    String(String),
    /// Any single-char punctuation or operator chunk, kept raw.
    Punct(char),
    /// Newline (one per `\n`).
    Newline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    byte: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub start_byte: usize,
    pub end_byte: usize,
    pub message: String,
}

impl LexError {
    pub fn into_parse_error(self, file_id: FileId) -> crate::span::ParseError {
        crate::span::ParseError::new(
            Span::new(file_id, self.start_byte as u32, self.end_byte as u32),
            self.message,
        )
    }
}

fn lex_error(from: Cursor, to: Cursor, message: String) -> LexError {
    LexError {
        start_byte: from.byte,
        end_byte: to.byte,
        message,
    }
}

/// Streaming lexer. Yields `Result<Spanned<Tok>, LexError>`; the parser
/// converts [`LexError`] into [`crate::span::ParseError`] with the file id.
pub struct Lexer<'a> {
    file_id: FileId,
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(file_id: FileId, src: &'a str) -> Self {
        Lexer { file_id, src, bytes: src.as_bytes(), pos: 0, done: false }
    }

    fn cursor(&self) -> Cursor {
        Cursor { byte: self.pos }
    }

    fn span(&self, from: Cursor, to: Cursor) -> Span {
        debug_assert!(from.byte <= to.byte);
        Span::new(self.file_id, from.byte as u32, to.byte as u32)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<(u8, u8)> {
        if self.pos + 1 < self.bytes.len() {
            Some((self.bytes[self.pos], self.bytes[self.pos + 1]))
        } else {
            None
        }
    }

    fn bump(&mut self, n: usize) {
        for _ in 0..n {
            if self.pos >= self.bytes.len() {
                return;
            }
            // Advance by one byte; Sail sources are UTF-8 and all structural
            // chars are ASCII, so byte/char sync holds at token boundaries.
            // Non-ASCII bytes inside idents/strings only shift columns by
            // bytes, which is documented and acceptable for a skeleton.
            self.pos += 1;
        }
    }

    fn span_text(&self, from: usize, to: usize) -> &'a str {
        &self.src[from..to]
    }

    /// Skip spaces, tabs, `\r`, line comments (`//...`) and block comments
    /// (`/* ... */`, nesting like the OCaml lexer — model files contain
    /// commented-out code with inner `/* ... */`). Returns true if a
    /// newline was crossed (callers needing exact line structure handle
    /// `\n` themselves — newlines are significant tokens here).
    fn skip_trivia(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => self.bump(1),
                Some(b'/') if self.peek2() == Some((b'/', b'/')) => {
                    while matches!(self.peek(), Some(c) if c != b'\n') {
                        self.bump(1);
                    }
                }
                Some(b'/') if self.peek2() == Some((b'/', b'*')) => {
                    let from = self.cursor();
                    let mut depth: u32 = 0;
                    loop {
                        // Peek two bytes without ever slicing past EOF.
                        let pair = if self.pos + 1 < self.bytes.len() {
                            Some((self.bytes[self.pos], self.bytes[self.pos + 1]))
                        } else {
                            None
                        };
                        match pair {
                            Some((b'/', b'*')) => {
                                depth += 1;
                                self.bump(2);
                            }
                            Some((b'*', b'/')) => {
                                depth -= 1;
                                self.bump(2);
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(_) => self.bump(1),
                            None => {
                                // Unterminated block comment.
                                if self.peek().is_some() {
                                    self.bump(1);
                                    continue;
                                }
                                let to = self.cursor();
                                return Err(lex_error(from, to, "unterminated block comment".to_owned()));
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn next_token(&mut self) -> Option<Result<Spanned<Tok>, LexError>> {
        if let Err(e) = self.skip_trivia() {
            return Some(Err(e));
        }
        let from = self.cursor();
        let c = self.peek()?;
        // Newline is a real token (defs are line-anchored at top level).
        if c == b'\n' {
            self.bump(1);
            let to = self.cursor();
            return Some(Ok(Spanned::new(Tok::Newline, self.span(from, to))));
        }
        // Pragma introducer `$name`.
        if c == b'$' {
            self.bump(1);
            let start = self.pos;
            while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
                self.bump(1);
            }
            let name = self.span_text(start, self.pos).to_owned();
            let to = self.cursor();
            if name.is_empty() {
                return Some(Err(lex_error(from, to, "stray `$` without pragma name".to_owned())));
            }
            return Some(Ok(Spanned::new(Tok::Pragma(name), self.span(from, to))));
        }
        // Identifier / keyword.
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = self.pos;
            while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'\'')) {
                self.bump(1);
            }
            let text = self.span_text(start, self.pos).to_owned();
            let to = self.cursor();
            return Some(Ok(Spanned::new(Tok::Ident(text), self.span(from, to))));
        }
        // Integer literal.
        if c.is_ascii_digit() {
            let start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9' | b'x' | b'b' | b'o' | b'A'..=b'F' | b'a'..=b'f' | b'_')) {
                self.bump(1);
            }
            let text = self.span_text(start, self.pos).to_owned();
            let to = self.cursor();
            return Some(Ok(Spanned::new(Tok::Int(text), self.span(from, to))));
        }
        // String literal "..." with backslash escapes.
        if c == b'"' {
            let start = self.pos;
            self.bump(1);
            let mut closed = false;
            while let Some(ch) = self.peek() {
                if ch == b'\\' {
                    self.bump(2);
                    continue;
                }
                if ch == b'"' {
                    self.bump(1);
                    closed = true;
                    break;
                }
                if ch == b'\n' {
                    break;
                }
                self.bump(1);
            }
            let to = self.cursor();
            if !closed {
                return Some(Err(lex_error(from, to, "unterminated string literal".to_owned())));
            }
            let text = self.span_text(start, self.pos).to_owned();
            return Some(Ok(Spanned::new(Tok::String(text), self.span(from, to))));
        }
        // Char literal 'x' — treat like a short string so a stray `'`
        // never breaks depth tracking.
        if c == b'\'' && self.pos + 2 < self.bytes.len() && self.bytes[self.pos + 2] == b'\'' {
            let start = self.pos;
            self.bump(3);
            let to = self.cursor();
            let text = self.span_text(start, self.pos).to_owned();
            return Some(Ok(Spanned::new(Tok::String(text), self.span(from, to))));
        }
        // Anything else: single punctuation char.
        self.bump(1);
        let to = self.cursor();
        Some(Ok(Spanned::new(Tok::Punct(c as char), self.span(from, to))))
    }
}

impl Iterator for Lexer<'_> {
    type Item = Result<Spanned<Tok>, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.next_token() {
            None => {
                self.done = true;
                None
            }
            other => other,
        }
    }
}
