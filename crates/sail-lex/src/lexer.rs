#![forbid(unsafe_code)]
//! Hand-written Sail2 lexer: bytes in, [`Token`] stream plus [`LexError`]s out.
//!
//! Reference behaviour is `sail/src/lib/lexer.mll` (OCaml). Deliberate
//! deviations are listed in `LEXER_NEEDED.md`.
//!
//! Error policy (fault tolerance): the lexer never fails. Recoverable
//! problems are pushed to [`LexResult::errors`]; depending on the case an
//! [`TokenKind::Error`] placeholder (skipped characters, reserved words,
//! stray closers) or a partial token (unterminated string/comment) is left in
//! the token stream so Lane 17 can skip and continue. [`lex`] always ends the
//! stream with [`TokenKind::Eof`].

use crate::{FileId, Fixity, Span, Token, TokenKind};
use sail_ast::Interner;

/// One recoverable lexing problem with its span and 1-based line/column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub span: Span,
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl LexError {
    #[must_use]
    pub fn new(span: Span, line: u32, col: u32, message: impl Into<String>) -> Self {
        LexError {
            span,
            line,
            col,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {} [{}..{}]",
            self.line, self.col, self.message, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for LexError {}

/// Output of [`lex`]: the token stream plus any recovery errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LexResult {
    pub tokens: Vec<Token>,
    pub errors: Vec<LexError>,
}

impl LexResult {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Tokens without documentation trivia (`DocBlock`/`DocLine`).
    #[must_use]
    pub fn code_tokens(&self) -> Vec<&Token> {
        self.tokens.iter().filter(|t| !t.is_doc()).collect()
    }
}

/// Lex `src` as Sail2 source for file `file`, interning every payload
/// ([`TokenKind`] text) into `interner`.
///
/// The same table should flow into the parser/AST interner (Lane 71
/// payload-intern): payload [`Symbol`][sail_ast::Symbol]s stay valid as
/// long as `interner` lives. [`TokenKind::Error`] messages are the only
/// payloads that stay `String` (they duplicate [`LexError::message`]).
#[must_use]
pub fn lex(file: FileId, src: &str, interner: &mut impl Interner) -> LexResult {
    Lexer::new(file, src, interner).run()
}

// ---------------------------------------------------------------------------
// Character classes (mirror the `ocamllex` definitions)
// ---------------------------------------------------------------------------

fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'?'
}

fn is_ident_start(b: u8) -> bool {
    is_letter(b) || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'?' || b == b'_' || b == b'\'' || b == b'#'
}

fn is_hex_digit(b: u8) -> bool {
    b.is_ascii_hexdigit() || b == b'_'
}

fn is_bin_digit(b: u8) -> bool {
    b == b'0' || b == b'1' || b == b'_'
}

fn is_oper_char(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'%'
            | b'&'
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'/'
            | b':'
            | b'<'
            | b'='
            | b'>'
            | b'@'
            | b'^'
            | b'|'
    )
}

fn is_wsc(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

struct Lexer<'a, 'i> {
    file: FileId,
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    line: u32,
    line_start: usize,
    tokens: Vec<Token>,
    errors: Vec<LexError>,
    interner: &'i mut dyn Interner,
}

impl<'a, 'i> Lexer<'a, 'i> {
    fn new(file: FileId, src: &'a str, interner: &'i mut impl Interner) -> Self {
        Lexer {
            file,
            src: src.as_bytes(),
            text: src,
            pos: 0,
            line: 1,
            line_start: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
            interner,
        }
    }

    /// Intern `text` into the caller's table.
    fn sym(&mut self, text: &str) -> sail_ast::Symbol {
        self.interner.intern(text)
    }

    fn run(mut self) -> LexResult {
        while self.pos < self.src.len() {
            self.skip_ws();
            if self.pos >= self.src.len() {
                break;
            }
            let start = self.pos;
            let b = self.src[self.pos];
            match b {
                b'/' if self.at(1) == Some(b'/') => {
                    if self.at(2) == Some(b'/') {
                        self.lex_doc_line(start);
                    } else {
                        self.skip_line_comment();
                    }
                }
                b'/' if self.at(1) == Some(b'*') => {
                    if self.at(2) == Some(b'!') {
                        self.lex_doc_block(start);
                    } else {
                        self.skip_block_comment(start, false);
                    }
                }
                b'/' if self.at(1) == Some(b'!') => {
                    // `/*!` without the `*` is not a doc opener; let the
                    // operator scanner deal with it (`/` then `!`).
                    self.lex_operator(start);
                }
                b'(' if self.at(1) == Some(b'*') => {
                    // Lane-spec `(* *)` comments: only when `(*` is followed
                    // by whitespace, otherwise this is `(` `*` (e.g. `(*x)`).
                    match self.at(2) {
                        Some(c) if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' => {
                            self.skip_block_comment(start, true);
                        }
                        _ => self.lex_punct(start),
                    }
                }
                b'"' => self.lex_string_or_multiline(start),
                b'$' => self.lex_directive(start),
                b'\'' => self.lex_tyvar_or_error(start),
                b'~' => {
                    self.bump();
                    let tilde = self.sym("~");
                    self.push(TokenKind::Id(tilde), start);
                }
                b if b.is_ascii_digit() => self.lex_number(start),
                b if is_ident_start(b) => self.lex_word(start),
                b if is_oper_char(b) => self.lex_operator(start),
                _ => self.lex_punct_or_error(start),
            }
        }
        let end = self.pos_u32();
        self.tokens
            .push(Token::new(TokenKind::Eof, Span::new(self.file, end, end)));
        LexResult {
            tokens: self.tokens,
            errors: self.errors,
        }
    }

    // -- primitives ----------------------------------------------------------

    fn at(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    /// Next Unicode scalar and its byte length (the input is valid UTF-8).
    fn peek_char(&self) -> Option<(char, usize)> {
        self.text
            .get(self.pos..)?
            .chars()
            .next()
            .map(|c| (c, c.len_utf8()))
    }

    fn bump(&mut self) {
        debug_assert!(self.pos < self.src.len());
        if self.src[self.pos] == b'\n' {
            self.line = self.line.saturating_add(1);
            self.pos += 1;
            self.line_start = self.pos;
        } else {
            self.pos += 1;
        }
    }

    fn pos_u32(&self) -> u32 {
        self.pos.min(u32::MAX as usize) as u32
    }

    fn span(&self, start: usize) -> Span {
        Span::new(
            self.file,
            start.min(u32::MAX as usize) as u32,
            self.pos_u32(),
        )
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        let span = self.span(start);
        self.tokens.push(Token::new(kind, span));
    }

    /// Record an error and leave an [`TokenKind::Error`] placeholder behind.
    fn push_error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        let message = message.into();
        let span = Span::new(
            self.file,
            start.min(u32::MAX as usize) as u32,
            end.min(u32::MAX as usize) as u32,
        );
        // Column of `start` on the current line (approximation is fine for
        // multi-line spans; exactness matters for single-char errors).
        let col = start.saturating_sub(self.line_start).min(u32::MAX as usize) as u32 + 1;
        self.errors
            .push(LexError::new(span, self.line, col, message.clone()));
        self.tokens
            .push(Token::new(TokenKind::Error(message), span));
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.src.get(self.pos) {
            match b {
                b'\n' => self.bump(),
                // Lone `\r` is plain whitespace (lenient); `\r\n` is handled
                // by skipping `\r` here and counting `\n` above.
                b' ' | b'\t' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    /// Scan an identifier body starting at `pos` (first char not consumed yet
    /// by the caller); returns the full identifier text.
    fn scan_ident(&mut self) -> &str {
        let start = self.pos;
        while let Some(&b) = self.src.get(self.pos) {
            if is_ident_continue(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        &self.text[start..self.pos]
    }

    // -- comments ------------------------------------------------------------

    /// `//` line comment: skipped trivia (lenient at EOF: no error).
    fn skip_line_comment(&mut self) {
        debug_assert!(self.text[self.pos..].starts_with("//"));
        self.pos += 2;
        while let Some(&b) = self.src.get(self.pos) {
            if b == b'\n' {
                break;
            }
            self.pos += 1;
        }
    }

    /// Nested block comment; `paren_star` selects `(* *)` vs `/* */`.
    /// Skipped trivia; unterminated input is an error (partial skip to EOF).
    fn skip_block_comment(&mut self, start: usize, paren_star: bool) {
        self.pos += 2; // consume opener
        let mut depth: u32 = 1;
        // Open/close pairs depend on the comment flavour.
        let (open0, open1, close0, close1): (u8, u8, u8, u8) = if paren_star {
            (b'(', b'*', b'*', b')')
        } else {
            (b'/', b'*', b'*', b'/')
        };
        while let Some(&b) = self.src.get(self.pos) {
            if b == open0 && self.at(1) == Some(open1) {
                depth += 1;
                self.pos += 2;
            } else if b == close0 && self.at(1) == Some(close1) {
                depth -= 1;
                self.pos += 2;
                if depth == 0 {
                    return;
                }
            } else if b == b'\n' {
                self.bump();
            } else {
                self.pos += 1;
            }
        }
        let msg = if paren_star {
            "Unbalanced comment: `(*` never closed by `*)`"
        } else {
            "Unbalanced comment: `/*` never closed by `*/`"
        };
        self.push_error(start, self.pos, msg);
    }

    /// `/*! … */` documentation block → [`TokenKind::DocBlock`] with the raw
    /// inner text (nesting with `/*!` and `/*` as in the OCaml lexer).
    fn lex_doc_block(&mut self, start: usize) {
        debug_assert!(self.text[self.pos..].starts_with("/*!"));
        self.pos += 3;
        let inner_start = self.pos;
        let mut depth: u32 = 1;
        while let Some(&b) = self.src.get(self.pos) {
            if b == b'/' && self.at(1) == Some(b'*') {
                depth += 1;
                self.pos += 2;
            } else if b == b'*' && self.at(1) == Some(b'/') {
                depth -= 1;
                self.pos += 2;
                if depth == 0 {
                    let inner = self.text[inner_start..self.pos - 2].to_string();
                    let sym = self.sym(&inner);
                    self.push(TokenKind::DocBlock(sym), start);
                    return;
                }
            } else if b == b'\n' {
                self.bump();
            } else {
                self.pos += 1;
            }
        }
        let inner = self.text[inner_start..self.pos].to_string();
        let sym = self.sym(&inner);
        self.push(TokenKind::DocBlock(sym), start);
        self.push_error(start, self.pos, "Unbalanced documentation comment");
    }

    /// `/// …` documentation lines → [`TokenKind::DocLine`]; continuation
    /// lines `
    /// …` are joined with `\n` (as in the OCaml lexer). The text keeps the
    /// space after `///`, exactly like the reference lexer.
    fn lex_doc_line(&mut self, start: usize) {
        debug_assert!(self.text[self.pos..].starts_with("///"));
        let mut content = String::new();
        loop {
            self.pos += 3; // consume `///`
            let line_start = self.pos;
            while let Some(&b) = self.src.get(self.pos) {
                if b == b'\n' {
                    break;
                }
                self.pos += 1;
            }
            content.push_str(&self.text[line_start..self.pos]);
            if self.pos >= self.src.len() {
                // EOF ends the comment leniently (the OCaml lexer raises).
                let sym = self.sym(&content);
                self.push(TokenKind::DocLine(sym), start);
                self.push_error(start, self.pos, "File ended in documentation comment");
                return;
            }
            // Look ahead past the newline and blanks for a `///` continuation.
            let mut ahead = self.pos + 1;
            while matches!(self.src.get(ahead), Some(b' ') | Some(b'\t')) {
                ahead += 1;
            }
            if self.text[ahead..].starts_with("///") {
                content.push('\n');
                self.bump(); // consume `\n` with line tracking
                while matches!(self.src.get(self.pos), Some(b' ') | Some(b'\t')) {
                    self.pos += 1;
                }
            } else {
                let sym = self.sym(&content);
                self.push(TokenKind::DocLine(sym), start);
                return;
            }
        }
    }

    // -- strings ---------------------------------------------------------------

    fn lex_string_or_multiline(&mut self, start: usize) {
        debug_assert!(self.src[self.pos] == b'"');
        if self.text[self.pos..].starts_with("\"\"\"") {
            self.lex_multiline_string(start);
        } else {
            self.pos += 1; // opening `"`
            let mut buf = String::new();
            let mut terminated = false;
            while let Some(&b) = self.src.get(self.pos) {
                match b {
                    b'"' => {
                        self.pos += 1;
                        terminated = true;
                        break;
                    }
                    b'\\' => {
                        if !self.lex_escape(&mut buf) {
                            break;
                        }
                    }
                    b'\n' => {
                        buf.push('\n');
                        self.bump();
                    }
                    _ => {
                        // Decoded as a full scalar so non-ASCII survives.
                        match self.peek_char() {
                            Some((c, len)) => {
                                buf.push(c);
                                self.pos += len;
                            }
                            None => break,
                        }
                    }
                }
            }
            let sym = self.sym(&buf);
            self.push(TokenKind::Str(sym), start);
            if !terminated {
                self.push_error(start, self.pos, "String literal not terminated");
            }
        }
    }

    /// `"""` opener already verified at `start`.
    fn lex_multiline_string(&mut self, start: usize) {
        self.pos += 3;
        // Opener line must hold only whitespace up to the newline (lenient:
        // skip to end of line and report instead of failing).
        let mut bad_opener = false;
        while let Some(&b) = self.src.get(self.pos) {
            if b == b'\n' {
                break;
            }
            if !is_wsc(b) {
                bad_opener = true;
            }
            self.pos += 1;
        }
        if self.pos >= self.src.len() {
            let sym = self.sym("");
            self.push(TokenKind::MultiLineStr(sym), start);
            self.push_error(start, self.pos, "Multiline string literal not terminated");
            return;
        }
        self.bump(); // consume `\n`
        if bad_opener {
            self.push_error(
                start,
                self.pos,
                "Characters after `\"\"\"` opener on the same line are not allowed",
            );
        }
        let mut buf = String::new();
        let mut terminated = false;
        while self.pos < self.src.len() {
            if self.text[self.pos..].starts_with("\"\"\"") {
                self.pos += 3;
                terminated = true;
                break;
            }
            match self.src[self.pos] {
                b'"' => {
                    buf.push('"');
                    self.pos += 1;
                }
                b'\\' => {
                    if !self.lex_escape(&mut buf) {
                        break;
                    }
                }
                b'\n' => {
                    buf.push('\n');
                    self.bump();
                }
                _ => match self.peek_char() {
                    Some((c, len)) => {
                        buf.push(c);
                        self.pos += len;
                    }
                    None => break,
                },
            }
        }
        if !terminated {
            // Fault tolerance: emit what we have plus an error.
            let (partial, _) = strip_lines(&buf, "");
            let sym = self.sym(&partial);
            self.push(TokenKind::MultiLineStr(sym), start);
            self.push_error(start, self.pos, "Multiline string literal not terminated");
            return;
        }
        // Text after the last `\n` is the indentation prefix; it must be
        // whitespace-only (OCaml `process_multiline`).
        let prefix = prefix_or_empty(&buf);
        let body = match buf.rfind('\n') {
            Some(i) => &buf[..i],
            None => "",
        };
        if !prefix.chars().all(|c| c == ' ' || c == '\t') {
            let (partial, _) = strip_lines(body, &prefix);
            let sym = self.sym(&partial);
            self.push(TokenKind::MultiLineStr(sym), start);
            self.push_error(
                start,
                self.pos,
                "Final line before `\"\"\"` delimiter in multi-line string must only contain whitespace",
            );
            return;
        }
        let (stripped, bad_line) = strip_lines(body, &prefix);
        let sym = self.sym(&stripped);
        self.push(TokenKind::MultiLineStr(sym), start);
        if bad_line {
            self.push_error(
                start,
                self.pos,
                "Line in multi-line string literal has an invalid indentation prefix",
            );
        }
    }

    /// Returns `false` when EOF was hit (caller stops the string loop).
    fn lex_escape(&mut self, buf: &mut String) -> bool {
        debug_assert!(self.src[self.pos] == b'\\');
        let err_start = self.pos;
        self.pos += 1;
        let Some(&c) = self.src.get(self.pos) else {
            self.push_error(err_start, self.pos, "String literal not terminated");
            return false;
        };
        match c {
            b'\\' => buf.push('\\'),
            b'"' => buf.push('"'),
            b'\'' => buf.push('\''),
            b'n' => buf.push('\n'),
            b't' => buf.push('\t'),
            b'b' => buf.push('\u{08}'),
            b'r' => buf.push('\r'),
            b'x' => {
                let hi = self.at(1);
                let lo = self.at(2);
                match (hi, lo) {
                    (Some(h), Some(l)) if h.is_ascii_hexdigit() && l.is_ascii_hexdigit() => {
                        let v = hex_val(h) * 16 + hex_val(l);
                        buf.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
                        self.pos += 3;
                        return true;
                    }
                    _ => {
                        self.push_error(
                            err_start,
                            self.pos + 1,
                            "String literal contains illegal backslash escape sequence",
                        );
                        buf.push('x');
                    }
                }
            }
            b if b.is_ascii_digit() => {
                let (d1, d2) = (self.at(1), self.at(2));
                match (d1, d2) {
                    (Some(a), Some(b2)) if a.is_ascii_digit() && b2.is_ascii_digit() => {
                        let v =
                            (b - b'0') as u32 * 100 + (a - b'0') as u32 * 10 + (b2 - b'0') as u32;
                        match char::from_u32(v) {
                            Some(ch) => buf.push(ch),
                            None => self.push_error(
                                err_start,
                                self.pos + 3,
                                "Invalid character code in string escape sequence",
                            ),
                        }
                        self.pos += 3;
                        return true;
                    }
                    _ => {
                        self.push_error(
                            err_start,
                            self.pos + 1,
                            "String literal contains illegal backslash escape sequence",
                        );
                        buf.push(b as char);
                    }
                }
            }
            b'\n' => {
                // Line continuation: backslash, newline, then blanks.
                self.bump();
                while let Some(&w) = self.src.get(self.pos) {
                    if w == b' ' || w == b'\t' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                return true;
            }
            _ => {
                self.push_error(
                    err_start,
                    self.pos + 1,
                    "String literal contains illegal backslash escape sequence",
                );
                // Fault tolerance: keep the escaped scalar literally.
                match self.peek_char() {
                    Some((c, len)) => {
                        buf.push(c);
                        self.pos += len;
                        return true;
                    }
                    None => return false,
                }
            }
        }
        // Simple one-char escapes: consume the escaped char.
        self.pos += 1;
        true
    }

    // -- directives ------------------------------------------------------------

    /// `$name …`, `$[attr`, `$name {`.
    fn lex_directive(&mut self, start: usize) {
        debug_assert!(self.src[self.pos] == b'$');
        self.pos += 1;
        if self.at(0) == Some(b'[') {
            self.pos += 1;
            if self.src.get(self.pos).is_some_and(|&b| is_ident_start(b)) {
                let name = self.scan_ident().to_string();
                let sym = self.sym(&name);
                self.push(TokenKind::Attribute(sym), start);
            } else {
                self.push_error(start, self.pos, "Expected attribute name after `$[`");
            }
            return;
        }
        let Some(&b) = self.src.get(self.pos) else {
            self.push_error(start, self.pos, "Expected directive name after `$`");
            return;
        };
        if !is_ident_start(b) {
            // Skip one char (ASCII or full UTF-8 sequence) and report.
            let len = self.text[self.pos..]
                .chars()
                .next()
                .map_or(1, |c| c.len_utf8());
            self.pos += len;
            self.push_error(start, self.pos, "Expected directive name after `$`");
            return;
        }
        let name = self.scan_ident().to_string();
        // `$name {` → structured pragma (the `{` is consumed, not emitted).
        let mut ahead = self.pos;
        while self.src.get(ahead).is_some_and(|&c| is_wsc(c)) {
            ahead += 1;
        }
        if self.src.get(ahead) == Some(&b'{') {
            self.pos = ahead + 1;
            let sym = self.sym(&name);
            self.push(TokenKind::StructuredPragma(sym), start);
            return;
        }
        // `$name value…` up to the end of the line.
        let name_sym = self.sym(&name);
        let mut value = String::new();
        loop {
            match self.src.get(self.pos) {
                None => {
                    let value_sym = self.sym(&unescape_pragma(&value));
                    self.push(
                        TokenKind::Pragma {
                            name: name_sym,
                            value: value_sym,
                        },
                        start,
                    );
                    self.push_error(start, self.pos, "File ended before newline in directive");
                    return;
                }
                Some(b'\n') => {
                    let value_sym = self.sym(&unescape_pragma(&value));
                    self.push(
                        TokenKind::Pragma {
                            name: name_sym,
                            value: value_sym,
                        },
                        start,
                    );
                    return;
                }
                Some(b'/') if self.at(1) == Some(b'/') => {
                    self.skip_line_comment();
                    let value_sym = self.sym(&unescape_pragma(&value));
                    self.push(
                        TokenKind::Pragma {
                            name: name_sym,
                            value: value_sym,
                        },
                        start,
                    );
                    return;
                }
                Some(b'/') if self.at(1) == Some(b'*') => {
                    let cstart = self.pos;
                    self.skip_block_comment_lenient(cstart);
                    // After a block comment only blanks may follow on the line.
                    let mut bad = false;
                    while let Some(&c) = self.src.get(self.pos) {
                        if c == b'\n' || self.pos >= self.src.len() {
                            break;
                        }
                        if !is_wsc(c) {
                            bad = true;
                        }
                        if c == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                    if bad {
                        self.push_error(
                            cstart,
                            self.pos,
                            "Directive continued after block comment",
                        );
                    }
                    let value_sym = self.sym(&unescape_pragma(&value));
                    self.push(
                        TokenKind::Pragma {
                            name: name_sym,
                            value: value_sym,
                        },
                        start,
                    );
                    return;
                }
                Some(_) => {
                    value.push(self.src[self.pos] as char);
                    self.pos += 1;
                }
            }
        }
    }

    /// Block comment skipper that never emits (for use inside directives).
    /// Returns `false` on unterminated input (error already recorded).
    fn skip_block_comment_lenient(&mut self, start: usize) -> bool {
        self.pos += 2;
        let mut depth: u32 = 1;
        while let Some(&b) = self.src.get(self.pos) {
            if b == b'/' && self.at(1) == Some(b'*') {
                depth += 1;
                self.pos += 2;
            } else if b == b'*' && self.at(1) == Some(b'/') {
                depth -= 1;
                self.pos += 2;
                if depth == 0 {
                    return true;
                }
            } else if b == b'\n' {
                self.bump();
            } else {
                self.pos += 1;
            }
        }
        self.push_error(start, self.pos, "Unbalanced comment");
        false
    }

    // -- words, type variables, numbers, operators, punctuation ------------------

    /// `infix[l|r] N op` fixity or keyword/identifier.
    fn lex_word(&mut self, start: usize) {
        let word = self.scan_ident().to_string();
        if word == "_" {
            // Tie with the identifier rule goes to `Under` (rule order).
            self.push(TokenKind::Under, start);
            return;
        }
        if word == "infix" || word == "infixl" || word == "infixr" {
            if let Some(fix) = self.try_fixity_suffix(&word) {
                self.push(fix, start);
                return;
            }
            let sym = self.sym(&word);
            self.push(TokenKind::Id(sym), start);
            return;
        }
        if word == "import" || word == "module" || word == "where" {
            self.push_error(start, self.pos, format!("`{word}` is a reserved keyword"));
            return;
        }
        match TokenKind::keyword(&word) {
            Some(kind) => self.push(kind, start),
            None => {
                let sym = self.sym(&word);
                self.push(TokenKind::Id(sym), start);
            }
        }
    }

    /// After `infix`/`infixl`/`infixr`: blanks, one digit, blanks, operator.
    fn try_fixity_suffix(&mut self, word: &str) -> Option<TokenKind> {
        let mut p = self.pos;
        while self.src.get(p).is_some_and(|&c| is_wsc(c)) {
            p += 1;
        }
        let level = *self.src.get(p)?;
        if !level.is_ascii_digit() {
            return None;
        }
        p += 1;
        // The digit must be followed by blanks (else e.g. `infix12` is an Id).
        if !self.src.get(p).is_some_and(|&c| is_wsc(c)) {
            return None;
        }
        while self.src.get(p).is_some_and(|&c| is_wsc(c)) {
            p += 1;
        }
        if !self.src.get(p).is_some_and(|&c| is_oper_char(c)) {
            return None;
        }
        // Commit: scan the operator with the usual rules.
        self.pos = p;
        let op = self.scan_operator_text();
        let op_sym = self.sym(&op);
        let fixity = match word {
            "infix" => Fixity::Infix,
            "infixl" => Fixity::InfixL,
            _ => Fixity::InfixR,
        };
        Some(TokenKind::Fixity {
            fixity,
            level: level - b'0',
            op: op_sym,
        })
    }

    fn lex_tyvar_or_error(&mut self, start: usize) {
        debug_assert!(self.src[self.pos] == b'\'');
        match self.at(1) {
            Some(b) if is_ident_start(b) => {
                let text = self.scan_ident().to_string();
                let sym = self.sym(&text);
                self.push(TokenKind::TyVar(sym), start);
            }
            _ => {
                self.pos += 1;
                self.push_error(start, self.pos, "Unexpected character: `'`");
            }
        }
    }

    fn lex_number(&mut self, start: usize) {
        let mut end = self.pos;
        while self.src.get(end).is_some_and(|&b| b.is_ascii_digit()) {
            end += 1;
        }
        let digits = &self.text[self.pos..end];
        // `0x…` / `0b…` (accept `0X`/`0B` too; OCaml only takes lowercase).
        if digits == "0"
            && matches!(
                self.src.get(end),
                Some(b'x') | Some(b'X') | Some(b'b') | Some(b'B')
            )
        {
            let marker = self.src[end];
            let is_hex = marker == b'x' || marker == b'X';
            let mut d = end + 1;
            while self.src.get(d).is_some_and(|&b| {
                if is_hex {
                    is_hex_digit(b)
                } else {
                    is_bin_digit(b)
                }
            }) {
                d += 1;
            }
            if d > end + 1 {
                let lit = self.text[end + 1..d].to_string();
                self.pos = d;
                let sym = self.sym(&lit);
                self.push(
                    if is_hex {
                        TokenKind::Hex(sym)
                    } else {
                        TokenKind::Bin(sym)
                    },
                    start,
                );
                return;
            }
            // No digits: fall through to `Num("0")` + identifier, as OCaml.
        }
        self.pos = end;
        // `2^` (blanks allowed between, no newline).
        if digits == "2" {
            let mut p = self.pos;
            while self.src.get(p).is_some_and(|&c| is_wsc(c)) {
                p += 1;
            }
            if self.src.get(p) == Some(&b'^') {
                self.pos = p + 1;
                self.push(TokenKind::TwoCaret, start);
                return;
            }
        }
        // `digits.digits` → Real (both sides required).
        if self.src.get(self.pos) == Some(&b'.')
            && self
                .src
                .get(self.pos + 1)
                .is_some_and(|b| b.is_ascii_digit())
        {
            self.pos += 1; // `.`
            while self.src.get(self.pos).is_some_and(|&b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            let text = self.text[start..self.pos].to_string();
            let sym = self.sym(&text);
            self.push(TokenKind::Real(sym), start);
            return;
        }
        let sym = self.sym(digits);
        self.push(TokenKind::Num(sym), start);
    }

    /// Operator entry: `Bidir`/`MinusGt`/`EqGt` take precedence over a longer
    /// operator match only when no further operator char follows (longest
    /// match wins, as in `ocamllex`). Single `.`/`..`/`*`/`:`/`::`/`|` and the
    /// `|]`/`|}` brackets beat the operator rule on ties (rule order).
    fn lex_operator(&mut self, start: usize) {
        let rest = &self.text[self.pos..];
        if rest.starts_with("*/") {
            self.pos += 2;
            self.push_error(start, self.pos, "Unbalanced comment: stray `*/`");
            return;
        }
        if rest.starts_with("<->") && !rest[3..].starts_with_oper_char() {
            self.pos += 3;
            self.push(TokenKind::Bidir, start);
            return;
        }
        if rest.starts_with("->") && !rest[2..].starts_with_oper_char() {
            self.pos += 2;
            self.push(TokenKind::MinusGt, start);
            return;
        }
        if rest.starts_with("=>") && !rest[2..].starts_with_oper_char() {
            self.pos += 2;
            self.push(TokenKind::EqGt, start);
            return;
        }
        for (pat, kind) in [("|]", TokenKind::RsquareBar), ("|}", TokenKind::RcurlyBar)] {
            if rest.starts_with(pat) {
                self.pos += pat.len();
                self.push(kind, start);
                return;
            }
        }
        if rest.starts_with("::") && !rest[2..].starts_with_oper_char() {
            self.pos += 2;
            self.push(TokenKind::ColonColon, start);
            return;
        }
        if rest.starts_with("..") && !rest[2..].starts_with_oper_char() {
            self.pos += 2;
            self.push(TokenKind::DotDot, start);
            return;
        }
        // Single-char punctuation that is also an operator char.
        if !rest.is_empty() && !rest[1..].starts_with_oper_char() {
            let kind = match rest.as_bytes()[0] {
                b':' => Some(TokenKind::Colon),
                b'.' => Some(TokenKind::Dot),
                b'*' => Some(TokenKind::Star),
                b'|' => Some(TokenKind::Bar),
                _ => None,
            };
            if let Some(kind) = kind {
                self.pos += 1;
                self.push(kind, start);
                return;
            }
        }
        let op = self.scan_operator_text();
        let kind = match op.as_str() {
            "=" => TokenKind::Eq,
            "-" => TokenKind::Minus,
            ":" => TokenKind::Colon,
            "@" => TokenKind::At,
            "^" => TokenKind::Caret,
            _ => TokenKind::Op(self.sym(&op)),
        };
        self.push(kind, start);
    }

    /// Maximal operator munch; stops before `//` and `/*` (comment starts).
    fn scan_operator_text(&mut self) -> String {
        let start = self.pos;
        while let Some(&b) = self.src.get(self.pos) {
            if !is_oper_char(b) {
                break;
            }
            if b == b'/' && matches!(self.at(1), Some(b'/') | Some(b'*')) {
                break;
            }
            self.pos += 1;
        }
        self.text[start..self.pos].to_string()
    }

    fn lex_punct(&mut self, start: usize) {
        let rest = &self.text[self.pos..];
        // Two/three-char tokens first (longest match).
        for (pat, kind) in [
            ("[|", TokenKind::LsquareBar),
            ("|]", TokenKind::RsquareBar),
            ("{|", TokenKind::LcurlyBar),
            ("|}", TokenKind::RcurlyBar),
            ("()", TokenKind::Unit),
            ("::", TokenKind::ColonColon),
            ("..", TokenKind::DotDot),
        ] {
            if let Some(after) = rest.strip_prefix(pat) {
                // `:::` / `...` lex as operators (longest match beats the
                // two-char rule in `ocamllex`); defer to the operator scanner.
                if (pat == "::" || pat == "..") && after.starts_with_oper_char() {
                    self.lex_operator(start);
                    return;
                }
                self.pos += pat.len();
                self.push(kind, start);
                return;
            }
        }
        let kind = match self.src[self.pos] {
            b'(' => TokenKind::Lparen,
            b')' => TokenKind::Rparen,
            b'[' => TokenKind::Lsquare,
            b']' => TokenKind::Rsquare,
            b'{' => TokenKind::Lcurly,
            b'}' => TokenKind::Rcurly,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semi,
            b'.' => TokenKind::Dot,
            b'|' => TokenKind::Bar,
            b'_' => TokenKind::Under,
            _ => unreachable!("punct dispatcher called on non-punctuation"),
        };
        self.pos += 1;
        self.push(kind, start);
    }

    /// Punctuation dispatcher plus fault tolerance for anything else
    /// (stray `*/`, non-ASCII, control characters, …).
    fn lex_punct_or_error(&mut self, start: usize) {
        if self.text[self.pos..].starts_with("*/") {
            self.pos += 2;
            self.push_error(start, self.pos, "Unbalanced comment: stray `*/`");
            return;
        }
        match self.src[self.pos] {
            b'(' | b')' | b'[' | b']' | b'{' | b'}' | b',' | b';' | b'.' | b'|' | b'_' => {
                self.lex_punct(start);
            }
            _ => {
                let ch = self.text[self.pos..].chars().next().unwrap_or('\u{FFFD}');
                let mut esc = String::new();
                for c in ch.escape_debug() {
                    esc.push(c);
                }
                self.pos += ch.len_utf8();
                self.push_error(start, self.pos, format!("Unexpected character: {esc}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

trait StartsWithOperChar {
    fn starts_with_oper_char(&self) -> bool;
}

impl StartsWithOperChar for str {
    fn starts_with_oper_char(&self) -> bool {
        self.as_bytes().first().is_some_and(|&b| is_oper_char(b))
    }
}

fn hex_val(b: u8) -> u32 {
    match b {
        b'0'..=b'9' => (b - b'0') as u32,
        b'a'..=b'f' => (b - b'a') as u32 + 10,
        b'A'..=b'F' => (b - b'A') as u32 + 10,
        _ => 0,
    }
}

fn prefix_or_empty(buf: &str) -> String {
    match buf.rfind('\n') {
        Some(i) => buf[i + 1..].to_string(),
        None => buf.to_string(),
    }
}

/// Strip the indentation `prefix` from every non-empty line of `body`.
/// Returns the joined text plus whether any line violated the prefix.
fn strip_lines(body: &str, prefix: &str) -> (String, bool) {
    let mut bad = false;
    let mut out = String::new();
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.is_empty() {
            continue;
        }
        match line.strip_prefix(prefix) {
            Some(rest) => out.push_str(rest),
            None => {
                bad = true;
                out.push_str(line);
            }
        }
    }
    (out, bad)
}

/// Resolve `\\`, `\"`, `\n`, … in `$directive` values (`Scanf.unescaped`).
fn unescape_pragma(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{08}'),
            Some(c @ ('\\' | '"' | '\'' | ' ')) => out.push(c),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
