//! Hand-written definition-level parser for Sail2.
//!
//! Strategy: single left-to-right scan over the source. Each top-level
//! definition starts at a line whose first token is a known starter
//! keyword (or `$` for pragmas). Headers are captured depth-aware
//! (parens/brackets/braces, comments and strings respected); bodies are
//! either brace-balanced blocks or run until the next definition start.
//! Bodies stay raw source text — full expression/type grammars are later
//! lanes (see `AST_NEEDED.md`).
//!
//! Properties:
//! * streaming per file: O(1) auxiliary state besides the emitted `Def`s;
//!   the input `&str` is walked once, never copied wholesale.
//! * thread-safe by construction: no globals, no shared mutable state —
//!   call [`crate::parse_files_parallel`] to parse files on many threads.
//! * `#![forbid(unsafe_code)]`-clean: pure safe Rust, byte/char boundary
//!   discipline documented below.
//!
//! UTF-8 note: structural scanning is byte-based and only inspects ASCII
//! bytes (`{`, `}`, `"`, `/`, ...). Multi-byte UTF-8 sequences never
//! contain ASCII bytes, so boundaries stay correct; line/column counts
//! count bytes for non-ASCII chars (documented, matches `lex` behaviour
//! of counting conservatively).

use crate::mini_ast::{Def, ParsedFile, PragmaArg, SpannedDef};
use crate::span::{ParseError, Span};

/// Keywords that start a new top-level definition when seen as the first
/// token of a line at nesting depth 0. Mirrors the `def:` rule of
/// `sail/src/lib/parser.mly` (skeleton subset). Deliberately excludes
/// `and` (extra function/mapping clauses, cf. `map_clauses.sail`) and
/// expression keywords (`if`, `then`, `else`, `match`, `foreach` bodies).
const STARTERS: &[&str] = &[
    "val",
    "function",
    "mapping",
    "impl",
    "let",
    "type",
    "struct",
    "enum",
    "union",
    "bitfield",
    "newtype",
    "constraint",
    "scattered",
    "register",
    "overload",
    "infix",
    "infixl",
    "infixr",
    "default",
    "outcome",
    "instantiation",
    "mutual",
    "private",
    "termination",
    "termination_measure",
    "clause",
];

fn is_starter(word: &str) -> bool {
    STARTERS.contains(&word)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'\''
}

#[derive(Debug, Clone, Copy)]
struct Cur {
    byte: usize,
    line: u32,
    col: u32,
}

struct Scanner<'a> {
    file: &'a str,
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Scanner<'a> {
    fn new(file: &'a str, src: &'a str) -> Self {
        Scanner { file, src, bytes: src.as_bytes(), pos: 0, line: 1, col: 1 }
    }

    fn cur(&self) -> Cur {
        Cur { byte: self.pos, line: self.line, col: self.col }
    }

    /// Save/restore the full cursor for speculative parsing.
    fn save(&self) -> (usize, u32, u32) {
        (self.pos, self.line, self.col)
    }

    fn restore(&mut self, s: (usize, u32, u32)) {
        self.pos = s.0;
        self.line = s.1;
        self.col = s.2;
    }

    fn span(&self, from: Cur, to: Cur) -> Span {
        Span::new(self.file, from.byte, to.byte, from.line, from.col, to.line, to.col)
    }

    fn err(&self, from: Cur, to: Cur, msg: &str) -> ParseError {
        ParseError::new(self.span(from, to), msg.to_owned())
    }

    fn eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos + off).copied()
    }

    fn bump(&mut self, n: usize) {
        for _ in 0..n {
            if self.eof() {
                return;
            }
            if self.bytes[self.pos] == b'\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.pos += 1;
        }
    }

    fn slice(&self, from: usize, to: usize) -> &'a str {
        &self.src[from..to]
    }

    /// Skip spaces, tabs, `\r` (not newlines). Returns true if anything moved.
    fn skip_inline_ws(&mut self) -> bool {
        let mut moved = false;
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.bump(1);
            moved = true;
        }
        moved
    }

    /// Skip one comment starting at the cursor (caller checked `//` or `/*`).
    /// Block comments are non-nesting, like the OCaml lexer.
    fn skip_comment(&mut self) -> Result<(), ParseError> {
        if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'/') {
            while matches!(self.peek(), Some(c) if c != b'\n') {
                self.bump(1);
            }
            Ok(())
        } else if self.peek() == Some(b'/') && self.peek_at(1) == Some(b'*') {
            let from = self.cur();
            self.bump(2);
            loop {
                match (self.peek(), self.peek_at(1)) {
                    (Some(b'*'), Some(b'/')) => {
                        self.bump(2);
                        return Ok(());
                    }
                    (Some(_), _) => self.bump(1),
                    (None, _) => {
                        let to = self.cur();
                        return Err(self.err(from, to, "unterminated block comment"));
                    }
                }
            }
        } else {
            Ok(())
        }
    }

    /// Skip inline whitespace + comments (not newlines).
    fn skip_trivia_inline(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_inline_ws();
            if self.peek() == Some(b'/')
                && matches!(self.peek_at(1), Some(b'/') | Some(b'*'))
            {
                self.skip_comment()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Skip everything insignificant including newlines and comments.
    fn skip_trivia_all(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_trivia_inline()?;
            if self.peek() == Some(b'\n') {
                self.bump(1);
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Read an identifier at the cursor. Returns text + span.
    fn read_ident(&mut self) -> Option<(String, Span)> {
        let from = self.cur();
        if !self.peek().is_some_and(is_ident_start) {
            return None;
        }
        let start = self.pos;
        while self.peek().is_some_and(is_ident_char) {
            self.bump(1);
        }
        let to = self.cur();
        Some((self.slice(start, self.pos).to_owned(), self.span(from, to)))
    }

    /// Read a `"..."` string at the cursor (including quotes), raw.
    fn read_string(&mut self) -> Result<String, ParseError> {
        let from = self.cur();
        debug_assert_eq!(self.peek(), Some(b'"'));
        let start = self.pos;
        self.bump(1);
        loop {
            match self.peek() {
                Some(b'\\') => self.bump(2),
                Some(b'"') => {
                    self.bump(1);
                    return Ok(self.slice(start, self.pos).to_owned());
                }
                Some(b'\n') | None => {
                    let to = self.cur();
                    return Err(self.err(from, to, "unterminated string literal"));
                }
                Some(_) => self.bump(1),
            }
        }
    }

    /// Skip a `"..."` string or `'x'` char literal at the cursor.
    fn skip_string(&mut self) -> Result<(), ParseError> {
        if self.peek() == Some(b'"') {
            self.read_string().map(|_| ())
        } else if self.peek() == Some(b'\'')
            && self.pos + 2 < self.bytes.len()
            && self.bytes[self.pos + 2] == b'\''
        {
            self.bump(3);
            Ok(())
        } else {
            Ok(())
        }
    }

    /// Capture from an opening `{` through its match, comment/string aware.
    /// Cursor must be at `{`. Returns raw text incl. braces.
    fn capture_balanced_braces(&mut self) -> Result<&'a str, ParseError> {
        let from = self.cur();
        debug_assert_eq!(self.peek(), Some(b'{'));
        let start = self.pos;
        let mut depth: u32 = 0;
        loop {
            match self.peek() {
                None => {
                    let to = self.cur();
                    return Err(self.err(from, to, "unterminated `{` block"));
                }
                Some(b'"') => self.skip_string()?,
                Some(b'\'')
                    if self.pos + 2 < self.bytes.len() && self.bytes[self.pos + 2] == b'\'' =>
                {
                    self.bump(3);
                }
                Some(b'/')
                    if matches!(self.peek_at(1), Some(b'/') | Some(b'*')) =>
                {
                    self.skip_comment()?;
                }
                Some(b'{') => {
                    depth += 1;
                    self.bump(1);
                }
                Some(b'}') => {
                    depth -= 1;
                    self.bump(1);
                    if depth == 0 {
                        return Ok(self.slice(start, self.pos));
                    }
                }
                Some(_) => self.bump(1),
            }
        }
    }

    /// Capture a header: scan until top-level `=` or `{` (depth counts
    /// `(`, `[`, `{`), or a blank-line / next-def line break. Returns
    /// (header_text, terminator) where terminator is `=`/`{`/end-kind.
    /// Cursor is left AT the terminator (or EOF).
    fn capture_header(&mut self) -> Result<(String, HeaderEnd), ParseError> {
        self.capture_header_inner(true)
    }

    /// Like [`Scanner::capture_header`], but never stops at `=`/`{`: only
    /// at end-of-line (with next-definition lookahead) or EOF. Used for
    /// the tail of `val` signatures after an embedded `{...}` block
    /// (backend attributes, `in {0,1}` constraint sets).
    fn capture_sig_rest(&mut self) -> Result<String, ParseError> {
        Ok(self.capture_header_inner(false)?.0)
    }

    fn capture_header_inner(&mut self, stop_at_eq_brace: bool) -> Result<(String, HeaderEnd), ParseError> {
        let start = self.pos;
        let mut paren: u32 = 0;
        let mut bracket: u32 = 0;
        let mut brace: u32 = 0;
        // A header may span lines while inside brackets, or via
        // continuation lines (next line does not start a new def).
        loop {
            match self.peek() {
                None => return Ok((self.slice(start, self.pos).trim().to_owned(), HeaderEnd::Eof)),
                Some(b'"') => {
                    self.skip_string()?;
                }
                Some(b'\'')
                    if self.pos + 2 < self.bytes.len() && self.bytes[self.pos + 2] == b'\'' =>
                {
                    self.bump(3);
                }
                Some(b'/')
                    if matches!(self.peek_at(1), Some(b'/') | Some(b'*')) =>
                {
                    self.skip_comment()?;
                }
                Some(b'(') => {
                    paren += 1;
                    self.bump(1);
                }
                Some(b')') => {
                    paren = paren.saturating_sub(1);
                    self.bump(1);
                }
                Some(b'[') => {
                    bracket += 1;
                    self.bump(1);
                }
                Some(b']') => {
                    bracket = bracket.saturating_sub(1);
                    self.bump(1);
                }
                Some(b'{') => {
                    if stop_at_eq_brace && paren == 0 && bracket == 0 && brace == 0 {
                        return Ok((self.slice(start, self.pos).trim().to_owned(), HeaderEnd::Brace));
                    }
                    brace += 1;
                    self.bump(1);
                }
                Some(b'}') => {
                    brace = brace.saturating_sub(1);
                    self.bump(1);
                }
                Some(b'=') if stop_at_eq_brace && paren == 0 && bracket == 0 && brace == 0 => {
                    // `==`, `=>`, `<=`, `>=`, `!=` are not header ends.
                    let n = self.peek_at(1);
                    if matches!(n, Some(b'=') | Some(b'>')) {
                        self.bump(1);
                    } else {
                        return Ok((self.slice(start, self.pos).trim().to_owned(), HeaderEnd::Eq));
                    }
                }
                Some(b'\n') if paren == 0 && bracket == 0 && brace == 0 => {
                    // Possible header end: check the next line.
                    let save = (self.pos, self.line, self.col);
                    self.bump(1);
                    // Peek next line's first word (skip inline ws/comments).
                    let mut p = self.pos;
                    while p < self.bytes.len()
                        && matches!(self.bytes[p], b' ' | b'\t' | b'\r')
                    {
                        p += 1;
                    }
                    let mut w = p;
                    while w < self.bytes.len()
                        && (self.bytes[w].is_ascii_alphanumeric()
                            || self.bytes[w] == b'_'
                            || self.bytes[w] == b'\'')
                    {
                        w += 1;
                    }
                    let word = &self.src[p..w];
                    let is_next_def = is_starter(word) || self.bytes.get(p) == Some(&b'$');
                    let is_blank = self.bytes.get(p) == Some(&b'\n');
                    if is_next_def || is_blank || p >= self.bytes.len() {
                        // Rewind to before the newline; caller skips trivia.
                        self.pos = save.0;
                        self.line = save.1;
                        self.col = save.2;
                        return Ok((self.slice(start, self.pos).trim().to_owned(), HeaderEnd::Eol));
                    }
                    // Continuation line: keep scanning.
                }
                Some(_) => self.bump(1),
            }
        }
    }

    /// Capture a brace-less body: scan until a line-start starter keyword
    /// at depth 0 (or `$`, or EOF). Cursor is left at the body end
    /// (start of the next def / EOF), trailing whitespace trimmed from
    /// the returned text.
    fn capture_until_next_def(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        let mut end = self.pos;
        let mut paren: u32 = 0;
        let mut bracket: u32 = 0;
        let mut brace: u32 = 0;
        let mut at_line_start = false;
        loop {
            match self.peek() {
                None => return Ok(self.slice(start, end).trim_end().to_owned()),
                Some(b'"') => {
                    self.skip_string()?;
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'\'')
                    if self.pos + 2 < self.bytes.len() && self.bytes[self.pos + 2] == b'\'' =>
                {
                    self.bump(3);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'/')
                    if matches!(self.peek_at(1), Some(b'/') | Some(b'*')) =>
                {
                    self.skip_comment()?;
                    // Comments don't extend the significant end.
                    at_line_start = false;
                }
                Some(b'(') => {
                    paren += 1;
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b')') => {
                    paren = paren.saturating_sub(1);
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'[') => {
                    bracket += 1;
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b']') => {
                    bracket = bracket.saturating_sub(1);
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'{') => {
                    brace += 1;
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'}') => {
                    if brace == 0 {
                        // Stray close at depth 0: end body, leave it.
                        return Ok(self.slice(start, end).trim_end().to_owned());
                    }
                    brace -= 1;
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
                Some(b'\n') => {
                    self.bump(1);
                    at_line_start = true;
                }
                Some(b' ' | b'\t' | b'\r') => {
                    self.bump(1);
                    // whitespace neither extends `end` nor resets line-start
                }
                Some(b'$') if at_line_start && paren == 0 && bracket == 0 && brace == 0 => {
                    return Ok(self.slice(start, end).trim_end().to_owned());
                }
                Some(c) if at_line_start && is_ident_start(c) && paren == 0 && bracket == 0 && brace == 0 => {
                    // Read the word and check starter membership.
                    let wstart = self.pos;
                    while self.peek().is_some_and(is_ident_char) {
                        self.bump(1);
                    }
                    if is_starter(self.slice(wstart, self.pos)) {
                        // Rewind to the first non-ws char of this line: the
                        // next definition starts here.
                        let mut line_begin = wstart;
                        while line_begin > 0
                            && matches!(self.bytes[line_begin - 1], b' ' | b'\t' | b'\r')
                        {
                            line_begin -= 1;
                        }
                        self.pos = line_begin;
                        // Resync column by recounting from the last newline
                        // (`line` is unchanged: we never crossed a newline).
                        let mut q = self.pos;
                        let mut c = 1u32;
                        while q > 0 && self.bytes[q - 1] != b'\n' {
                            q -= 1;
                            c += 1;
                        }
                        self.col = c;
                        return Ok(self.slice(start, end).trim_end().to_owned());
                    }
                    end = self.pos;
                    at_line_start = false;
                }
                Some(_) => {
                    self.bump(1);
                    end = self.pos;
                    at_line_start = false;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderEnd {
    Eq,
    Brace,
    Eol,
    Eof,
}

/// Parse one `.sail` file. Always succeeds at the file level unless a
/// lexical/structural error occurs; the result is wrapped in
/// `file_start`/`file_end` pragma markers (like OCaml `initial_check.ml`).
pub fn parse_file(path: &str, src: &str) -> Result<ParsedFile, ParseError> {
    let mut sc = Scanner::new(path, src);
    let mut defs: Vec<SpannedDef> = Vec::new();

    defs.push(SpannedDef::new(
        Def::Pragma { name: "file_start".to_owned(), arg: PragmaArg::Line(path.to_owned()) },
        Span::point(path, 0, 1, 1),
    ));

    loop {
        sc.skip_trivia_all()?;
        if sc.eof() {
            break;
        }
        let def_start = sc.cur();
        if sc.peek() == Some(b'$') {
            let (def, end) = parse_pragma(&mut sc)?;
            defs.push(SpannedDef::new(def, sc.span(def_start, end)));
            continue;
        }
        let Some((kw, _kw_span)) = sc.read_ident() else {
            let to = sc.cur();
            // Skip one stray char with an error: unbalanced `}` etc.
            if sc.peek() == Some(b'}') {
                return Err(sc.err(def_start, to, "unbalanced `}` at top level"));
            }
            let ch = sc.peek().unwrap_or(b'?') as char;
            return Err(sc.err(def_start, to, &format!("expected definition keyword, found `{ch}`")));
        };
        if kw == "private" {
            // Visibility prefix: parse the real def, extend its span.
            sc.skip_trivia_all()?;
            let inner_start = sc.cur();
            let Some((ikw, _)) = sc.read_ident() else {
                let to = sc.cur();
                return Err(sc.err(def_start, to, "expected definition after `private`"));
            };
            let (def, end) = parse_keyword_def(&mut sc, path, &ikw, inner_start)?;
            let _ = inner_start;
            defs.push(SpannedDef::new(def, sc.span(def_start, end)));
            continue;
        }
        if !is_starter(&kw) {
            let to = sc.cur();
            return Err(sc.err(
                def_start,
                to,
                &format!("expected definition keyword, found `{kw}` (line must start a definition)"),
            ));
        }
        let (def, end) = parse_keyword_def(&mut sc, path, &kw, def_start)?;
        defs.push(SpannedDef::new(def, sc.span(def_start, end)));
    }

    let eof = sc.cur();
    defs.push(SpannedDef::new(
        Def::Pragma { name: "file_end".to_owned(), arg: PragmaArg::Line(path.to_owned()) },
        Span::point(path, eof.byte, eof.line, eof.col),
    ));

    Ok(ParsedFile { path: path.to_owned(), defs })
}

/// Parse `$name ...`, `$name{...}` or `$[name ...]` at the cursor
/// (cursor is AT `$`). Returns the def + end cursor.
fn parse_pragma(sc: &mut Scanner<'_>) -> Result<(Def, Cur), ParseError> {
    let from = sc.cur();
    sc.bump(1); // `$`
    if sc.peek() == Some(b'[') {
        // Attribute form `$[name kvs...]`: preserve raw inside.
        sc.bump(1);
        let start = sc.pos;
        let mut depth: u32 = 1;
        loop {
            match sc.peek() {
                None => {
                    let to = sc.cur();
                    return Err(sc.err(from, to, "unterminated `$[` attribute"));
                }
                Some(b'"') => {
                    sc.skip_string()?;
                }
                Some(b'[') => {
                    depth += 1;
                    sc.bump(1);
                }
                Some(b']') => {
                    depth -= 1;
                    sc.bump(1);
                    if depth == 0 {
                        break;
                    }
                }
                Some(_) => sc.bump(1),
            }
        }
        let inside = sc.slice(start, sc.pos - 1).trim().to_owned();
        let mut parts = inside.splitn(2, |c: char| c.is_whitespace());
        let name = parts.next().unwrap_or("").to_owned();
        let rest = parts.next().unwrap_or("").trim().to_owned();
        if name.is_empty() {
            let to = sc.cur();
            return Err(sc.err(from, to, "empty `$[]` attribute"));
        }
        let end = sc.cur();
        // Skip rest of line.
        while matches!(sc.peek(), Some(c) if c != b'\n') {
            sc.bump(1);
        }
        let _ = end;
        return Ok((
            Def::Pragma { name, arg: PragmaArg::Structured(rest) },
            sc.cur(),
        ));
    }
    // `$name`
    let name_start = sc.pos;
    while sc.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
        sc.bump(1);
    }
    let name = sc.slice(name_start, sc.pos).to_owned();
    if name.is_empty() {
        let to = sc.cur();
        return Err(sc.err(from, to, "stray `$` without pragma name"));
    }
    sc.skip_trivia_inline()?;
    if sc.peek() == Some(b'{') {
        let raw = sc.capture_balanced_braces()?.to_owned();
        let inner = raw[1..raw.len() - 1].trim().to_owned();
        // Skip rest of line after structured pragma.
        sc.skip_trivia_inline()?;
        let end = sc.cur();
        return Ok((Def::Pragma { name, arg: PragmaArg::Structured(inner) }, end));
    }
    // Rest-of-line argument.
    let start = sc.pos;
    while matches!(sc.peek(), Some(c) if c != b'\n') {
        sc.bump(1);
    }
    let arg = sc.slice(start, sc.pos).trim().to_owned();
    Ok((Def::Pragma { name, arg: PragmaArg::Line(arg) }, sc.cur()))
}

/// Parse a definition whose starter keyword was just consumed.
fn parse_keyword_def(
    sc: &mut Scanner<'_>,
    _path: &str,
    kw: &str,
    _def_start: Cur,
) -> Result<(Def, Cur), ParseError> {
    match kw {
        "val" => {
            sc.skip_trivia_all()?;
            // Optional quoted operator name: val "op" : ...
            let name = if sc.peek() == Some(b'"') {
                let raw = sc.read_string()?;
                raw[1..raw.len() - 1].to_owned()
            } else {
                let (head, end_kind) = sc.capture_header()?;
                // Name = first ident in header.
                let nm = first_ident(&head).unwrap_or_default();
                return Ok((Def::Val { name: nm, sig_src: finish_val_sig(sc, head, end_kind)? }, sc.cur()));
            };
            let (head, end_kind) = sc.capture_header()?;
            // header starts with `: ...`; sig = name + header
            let sig_src = format!("{name}{}", head.trim_start());
            let sig_src = finish_val_sig(sc, sig_src, end_kind)?;
            Ok((Def::Val { name, sig_src }, sc.cur()))
        }
        "function" | "mapping" | "impl" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let name = first_ident(&head).unwrap_or_default();
            let (head_src, mut body_src) = finish_eq_body(sc, head, end_kind)?;
            // Extra clauses: `and pat = body` (funcls2/mapcls in the OCaml
            // grammar) continue the same definition.
            loop {
                let save = sc.save();
                sc.skip_trivia_all()?;
                let is_and = sc.peek().is_some_and(is_ident_start)
                    && {
                        let p = sc.pos;
                        let mut q = p;
                        while q < sc.bytes.len() && is_ident_char(sc.bytes[q]) {
                            q += 1;
                        }
                        &sc.src[p..q] == "and"
                    };
                if !is_and {
                    sc.restore(save);
                    break;
                }
                // Consume `and` + clause header + body, append raw.
                while sc.peek().is_some_and(is_ident_char) {
                    sc.bump(1);
                }
                sc.skip_trivia_all()?;
                let (chead, cend) = sc.capture_header()?;
                let (_, cbody) = finish_eq_body(sc, chead.clone(), cend)?;
                body_src.push_str("\nand ");
                body_src.push_str(&chead);
                if !cbody.is_empty() {
                    body_src.push_str(" = ");
                    body_src.push_str(&cbody);
                }
            }
            let def = match kw {
                "function" => Def::Fundef { name, head_src, body_src },
                "mapping" => Def::Mapdef { name, head_src, body_src },
                _ => Def::Impl { name, head_src, body_src },
            };
            // Function-name-as-first-ident works for `name(args)...`; keep raw head too.
            Ok((def, sc.cur()))
        }
        "type" | "struct" | "enum" | "union" | "bitfield" | "newtype" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let name = first_ident(&head).unwrap_or_default();
            let (head_src, body_src) = finish_eq_body(sc, head, end_kind)?;
            let body_src = if body_src.is_empty() { head_src.clone() } else { body_src };
            let _ = head_src;
            Ok((Def::Type { name, body_src }, sc.cur()))
        }
        "let" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let (pat_src, body_src) = finish_eq_body(sc, head, end_kind)?;
            Ok((Def::Let { pat_src, body_src }, sc.cur()))
        }
        "register" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let name = first_ident(&head).unwrap_or_default();
            // Split `name : type [= init]`.
            let (type_src, init_src) = match end_kind {
                HeaderEnd::Eq => {
                    let t = head;
                    sc.skip_trivia_all()?;
                    let init = if sc.eof() { String::new() } else { sc.capture_until_next_def()? };
                    let init = if init.is_empty() { None } else { Some(init) };
                    (t, init)
                }
                _ => (head, None),
            };
            Ok((Def::Register { name, type_src, init_src }, sc.cur()))
        }
        "overload" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let name = first_ident(&head).unwrap_or_default();
            let (head_src, body_src) = finish_eq_body(sc, head, end_kind)?;
            let ids_src = if body_src.is_empty() { head_src } else { body_src };
            let ids = idents_in(&ids_src);
            Ok((Def::Overload { name, ids }, sc.cur()))
        }
        "infix" | "infixl" | "infixr" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let _ = end_kind;
            let mut it = head.split_whitespace();
            let level = it.next().unwrap_or("").to_owned();
            let name = it.collect::<Vec<_>>().join(" ");
            Ok((Def::Fixity { prec: kw.to_owned(), level, name }, sc.cur()))
        }
        "default" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let _ = end_kind;
            let mut it = head.split_whitespace();
            let kind = it.next().unwrap_or("").to_owned();
            let value = it.collect::<Vec<_>>().join(" ");
            Ok((Def::Default { kind, value }, sc.cur()))
        }
        "scattered" => {
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let (head_src, body_src) = finish_eq_body(sc, head, end_kind)?;
            Ok((Def::Scattered { head_src, body_src }, sc.cur()))
        }
        "constraint" | "outcome" | "instantiation" | "termination" | "termination_measure" | "mutual" | "clause" => {
            // Raw-capture defs: header + optional `=`-body.
            sc.skip_trivia_all()?;
            let (head, end_kind) = sc.capture_header()?;
            let (head_src, body_src) = finish_eq_body(sc, head, end_kind)?;
            let src = if body_src.is_empty() { head_src } else { format!("{head_src} = {body_src}") };
            let def = match kw {
                "constraint" => Def::Constraint { src },
                "outcome" => Def::Outcome { src },
                "instantiation" => Def::Instantiation { src },
                "termination" | "termination_measure" => Def::Measure { src },
                "mutual" => Def::Other { keyword: kw.to_owned(), src },
                _ => Def::Other { keyword: kw.to_owned(), src },
            };
            Ok((def, sc.cur()))
        }
        _ => {
            // Unreachable: caller checks is_starter. Kept for robustness.
            let from = sc.cur();
            let to = sc.cur();
            let _ = from;
            Err(ParseError::new(to_byte_span(sc, to), format!("unhandled definition keyword `{kw}`")))
        }
    }
}

/// Build the `val` signature: header already captured; if it ended at `=`
/// (val-with-default? rare) append the body, else done. Multi-line handled
/// by [`Scanner::capture_header`].
fn finish_val_sig(sc: &mut Scanner<'_>, head: String, end_kind: HeaderEnd) -> Result<String, ParseError> {
    match end_kind {
        HeaderEnd::Eq => {
            sc.bump(1);
            sc.skip_trivia_all()?;
            if sc.peek() == Some(b'{') || sc.peek() == Some(b'p') && peek_word(sc, "pure") {
                // Backend-attribute block `{ ... }` (optionally after `pure`):
                // the type ascription `: ...` follows on later lines.
                let mut sig = format!("{head} = ");
                if peek_word(sc, "pure") {
                    while sc.peek().is_some_and(is_ident_char) {
                        sc.bump(1);
                    }
                    sig.push_str("pure ");
                }
                sc.skip_trivia_all()?;
                if sc.peek() == Some(b'{') {
                    sig.push_str(sc.capture_balanced_braces()?);
                    sc.skip_trivia_all()?;
                    if sc.peek() == Some(b':') {
                        let rest = sc.capture_sig_rest()?;
                        if !rest.trim().is_empty() {
                            sig.push_str(&rest);
                        }
                    }
                    return Ok(sig);
                }
                // `pure` without a block (should not happen): fall through.
                let body = sc.capture_until_next_def()?;
                sig.push_str(&body);
                return Ok(sig);
            }
            let body = sc.capture_until_next_def()?;
            Ok(format!("{head} = {body}"))
        }
        HeaderEnd::Brace => {
            let raw = sc.capture_balanced_braces()?.to_owned();
            // The `{...}` was embedded in the signature (e.g. `'n in
            // {0,1}` or a backend block): the signature continues.
            sc.skip_trivia_all()?;
            let rest = sc.capture_sig_rest()?;
            if rest.trim().is_empty() {
                Ok(format!("{head} {raw}"))
            } else {
                Ok(format!("{head} {raw}{rest}"))
            }
        }
        HeaderEnd::Eol | HeaderEnd::Eof => Ok(head),
    }
}

/// True if the identifier at the cursor is exactly `word`.
fn peek_word(sc: &Scanner<'_>, word: &str) -> bool {
    let mut q = sc.pos;
    while q < sc.bytes.len() && is_ident_char(sc.bytes[q]) {
        q += 1;
    }
    &sc.src[sc.pos..q] == word
}

/// After a header capture, consume the `=`-body (if any). Returns
/// (header, body); body is empty when the header ended without `=`/`{`.
fn finish_eq_body(
    sc: &mut Scanner<'_>,
    head: String,
    end_kind: HeaderEnd,
) -> Result<(String, String), ParseError> {
    match end_kind {
        HeaderEnd::Eq => {
            sc.bump(1); // `=`
            sc.skip_trivia_all()?;
            if sc.peek() == Some(b'{') {
                let body = sc.capture_balanced_braces()?.to_owned();
                Ok((head, body))
            } else if sc.eof() {
                Ok((head, String::new()))
            } else {
                Ok((head, sc.capture_until_next_def()?))
            }
        }
        HeaderEnd::Brace => {
            let raw = sc.capture_balanced_braces()?.to_owned();
            Ok((head, raw))
        }
        HeaderEnd::Eol | HeaderEnd::Eof => Ok((head, String::new())),
    }
}

fn to_byte_span(sc: &Scanner<'_>, cur: Cur) -> Span {
    sc.span(cur, cur)
}

/// First identifier-like word in `s` (skips punctuation).
fn first_ident(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if is_ident_start(b[i]) {
            let mut j = i + 1;
            while j < b.len() && is_ident_char(b[j]) {
                j += 1;
            }
            return Some(s[i..j].to_owned());
        }
        i += 1;
    }
    None
}

/// All identifier-like words in `s`.
fn idents_in(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if is_ident_start(b[i]) {
            let mut j = i + 1;
            while j < b.len() && is_ident_char(b[j]) {
                j += 1;
            }
            out.push(s[i..j].to_owned());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}
