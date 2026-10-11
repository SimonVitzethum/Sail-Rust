//! Hand-written lexer for the Sail grammar port.
//!
//! Mirrors the token set of `sail/src/lib/lexer.mll` (`%token` declarations
//! in `parser.mly`), minus source positions: tokens carry byte offsets only.
//! Payloads that are constant in the OCaml lexer (`=`, `=>`, `()`, `:`)
//! become unit variants here.

#![forbid(unsafe_code)]

use crate::ast::FixAssoc;

/// Lexical error with byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub offset: usize,
    pub message: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lex error at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for LexError {}

/// Sail token. Keyword/punctuation variants are unit; only tokens with
/// content carry payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    And,
    As,
    Assert,
    Bitzero,
    Bitone,
    By,
    Match,
    Config,
    Clause,
    Dec,
    Default,
    Effect,
    End,
    OperatorKw,
    Enum,
    Else,
    Exit,
    Cast,
    False,
    Forall,
    Foreach,
    Function,
    Mapping,
    Overload,
    Throw,
    Try,
    Catch,
    If,
    In,
    Inc,
    Let,
    Var,
    Ref,
    Int,
    Nat,
    Order,
    Bool,
    Pure,
    Impure,
    Monadic,
    Register,
    Return,
    Scattered,
    Sizeof,
    Constraint,
    Constant,
    Struct,
    Then,
    True,
    Type,
    Typedef,
    Undefined,
    Union,
    Newtype,
    With,
    When,
    Val,
    Outcome,
    Instantiation,
    Impl,
    Private,
    Repeat,
    Until,
    While,
    Do,
    Mutual,
    Bitfield,
    Configuration,
    TerminationMeasure,
    Forwards,
    Backwards,
    From,
    To,
    Downto,
    InternalPLet,
    InternalReturn,
    InternalAssume,
    Bar,
    Comma,
    Dot,
    Minus,
    Semi,
    Under,
    DotDot,
    At,
    ColonColon,
    Caret,
    Star,
    Lcurly,
    Rcurly,
    Lparen,
    Rparen,
    Lsquare,
    Rsquare,
    LcurlyBar,
    RcurlyBar,
    LsquareBar,
    RsquareBar,
    MinusGt,
    Bidir,
    TwoCaret,
    Eq,
    EqGt,
    Unit,
    Colon,
    Id(String),
    TyVar(String),
    Num(String),
    Bin(String),
    Hex(String),
    Real(String),
    Str(String),
    OpId(String),
    DocBlock(String),
    DocLine(String),
    Pragma(String, String),
    StructPragma(String),
    Attr(String),
    Fixity(FixAssoc, u8, String),
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '?'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '#' | '?')
}

fn is_oper_char(c: char) -> bool {
    matches!(c, '!' | '%' | '&' | '*' | '+' | '-' | '.' | '/' | ':' | '<' | '=' | '>' | '@' | '^' | '|')
}

fn keyword(word: &str) -> Option<Tok> {
    let t = match word {
        "_" => Tok::Under,
        "and" => Tok::And,
        "as" => Tok::As,
        "assert" => Tok::Assert,
        "bitzero" => Tok::Bitzero,
        "bitone" => Tok::Bitone,
        "by" => Tok::By,
        "match" => Tok::Match,
        "config" => Tok::Config,
        "clause" => Tok::Clause,
        "dec" => Tok::Dec,
        "default" => Tok::Default,
        "effect" => Tok::Effect,
        "end" => Tok::End,
        "operator" => Tok::OperatorKw,
        "enum" => Tok::Enum,
        "else" => Tok::Else,
        "exit" => Tok::Exit,
        "cast" => Tok::Cast,
        "false" => Tok::False,
        "forall" => Tok::Forall,
        "foreach" => Tok::Foreach,
        "function" => Tok::Function,
        "mapping" => Tok::Mapping,
        "overload" => Tok::Overload,
        "throw" => Tok::Throw,
        "try" => Tok::Try,
        "catch" => Tok::Catch,
        "if" => Tok::If,
        "in" => Tok::In,
        "inc" => Tok::Inc,
        "let" => Tok::Let,
        "var" => Tok::Var,
        "ref" => Tok::Ref,
        "Int" => Tok::Int,
        "Nat" => Tok::Nat,
        "Order" => Tok::Order,
        "Bool" => Tok::Bool,
        "pure" => Tok::Pure,
        "impure" => Tok::Impure,
        "monadic" => Tok::Monadic,
        "register" => Tok::Register,
        "return" => Tok::Return,
        "scattered" => Tok::Scattered,
        "sizeof" => Tok::Sizeof,
        "constraint" => Tok::Constraint,
        "constant" => Tok::Constant,
        "struct" => Tok::Struct,
        "then" => Tok::Then,
        "true" => Tok::True,
        "Type" => Tok::Type,
        "type" => Tok::Typedef,
        "undefined" => Tok::Undefined,
        "union" => Tok::Union,
        "newtype" => Tok::Newtype,
        "with" => Tok::With,
        "when" => Tok::When,
        "val" => Tok::Val,
        "outcome" => Tok::Outcome,
        "instantiation" => Tok::Instantiation,
        "impl" => Tok::Impl,
        "private" => Tok::Private,
        "repeat" => Tok::Repeat,
        "until" => Tok::Until,
        "while" => Tok::While,
        "do" => Tok::Do,
        "mutual" => Tok::Mutual,
        "bitfield" => Tok::Bitfield,
        "configuration" => Tok::Configuration,
        "termination_measure" => Tok::TerminationMeasure,
        "forwards" => Tok::Forwards,
        "backwards" => Tok::Backwards,
        "from" => Tok::From,
        "to" => Tok::To,
        "downto" => Tok::Downto,
        "internal_plet" => Tok::InternalPLet,
        "internal_return" => Tok::InternalReturn,
        "internal_assume" => Tok::InternalAssume,
        _ => return None,
    };
    Some(t)
}

struct Cursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(s: &'a str) -> Self {
        Self { src: s.as_bytes(), pos: 0 }
    }

    fn err(&self, message: impl Into<String>) -> LexError {
        LexError { offset: self.pos, message: message.into() }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    fn bump(&mut self, n: usize) {
        self.pos += n;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }
}

/// Tokenize `src` into `(start, token, end)` triples with byte offsets.
pub fn tokenize(src: &str) -> Result<Vec<(usize, Tok, usize)>, LexError> {
    let mut c = Cursor::new(src);
    let mut out = Vec::new();
    loop {
        c.skip_ws();
        let start = c.pos;
        let Some(b) = c.peek() else { break };
        // ocamllex uses longest match across punctuation and operators.
        // `==`, `>=`, `^^` and `/` must not be split into punctuation tokens.
        if is_oper_char(b as char)
            && !c.starts_with("//") && !c.starts_with("/*")
            && !c.starts_with("|]") && !c.starts_with("|}")
        {
            lex_operator(&mut c, src, start, &mut out)?;
            continue;
        }
        match b {
            b',' => {
                c.bump(1);
                out.push((start, Tok::Comma, c.pos));
            }
            b';' => {
                c.bump(1);
                out.push((start, Tok::Semi, c.pos));
            }
            b'{' => {
                if c.starts_with("{|") {
                    c.bump(2);
                    out.push((start, Tok::LcurlyBar, c.pos));
                } else {
                    c.bump(1);
                    out.push((start, Tok::Lcurly, c.pos));
                }
            }
            b'}' => {
                c.bump(1);
                out.push((start, Tok::Rcurly, c.pos));
            }
            b'(' => {
                if c.starts_with("()") {
                    c.bump(2);
                    out.push((start, Tok::Unit, c.pos));
                } else {
                    c.bump(1);
                    out.push((start, Tok::Lparen, c.pos));
                }
            }
            b')' => {
                c.bump(1);
                out.push((start, Tok::Rparen, c.pos));
            }
            b'[' => {
                if c.starts_with("[|") {
                    c.bump(2);
                    out.push((start, Tok::LsquareBar, c.pos));
                } else {
                    c.bump(1);
                    out.push((start, Tok::Lsquare, c.pos));
                }
            }
            b']' => {
                c.bump(1);
                out.push((start, Tok::Rsquare, c.pos));
            }
            b'|' => {
                if c.starts_with("|]") {
                    c.bump(2);
                    out.push((start, Tok::RsquareBar, c.pos));
                } else if c.starts_with("|}") {
                    c.bump(2);
                    out.push((start, Tok::RcurlyBar, c.pos));
                } else {
                    c.bump(1);
                    out.push((start, Tok::Bar, c.pos));
                }
            }
            b'/' => {
                if c.starts_with("//") {
                    if c.starts_with("///") {
                        lex_doc_line(&mut c, src, start, &mut out)?;
                    } else {
                        // line comment: skip
                        while !matches!(c.peek(), None | Some(b'\n')) {
                            c.bump(1);
                        }
                    }
                } else if c.starts_with("/*") {
                    if c.starts_with("/*!") {
                        lex_doc_block(&mut c, src, start, &mut out)?;
                    } else {
                        skip_block_comment(&mut c)?;
                    }
                } else {
                    lex_operator(&mut c, src, start, &mut out)?;
                }
            }
            b'$' => lex_directive(&mut c, src, start, &mut out)?,
            b'"' => {
                if c.starts_with("\"\"\"") {
                    lex_multiline_string(&mut c, src, start, &mut out)?;
                } else {
                    let s = lex_string(&mut c)?;
                    out.push((start, Tok::Str(s), c.pos));
                }
            }
            b'~' => {
                c.bump(1);
                out.push((start, Tok::Id("~".to_string()), c.pos));
            }
            b'\'' => {
                // type variable `'a...`
                let mut end = c.pos + 1;
                if !c.src.get(end).is_some_and(|&x| is_ident_start(x as char)) {
                    return Err(c.err("unexpected character: '"));
                }
                while c.src.get(end).is_some_and(|&x| is_ident_char(x as char)) {
                    end += 1;
                }
                let v = src[c.pos..end].to_string();
                c.bump(end - c.pos);
                out.push((start, Tok::TyVar(v), c.pos));
            }
            b'0'..=b'9' => {
                // `2 ^` (with optional spaces) is `TwoCaret`, like ocamllex.
                if b == b'2' {
                    let mut k = c.pos + 1;
                    while c.src.get(k) == Some(&b' ') || c.src.get(k) == Some(&b'\t') {
                        k += 1;
                    }
                    if c.src.get(k) == Some(&b'^') {
                        c.bump(k + 1 - c.pos);
                        out.push((start, Tok::TwoCaret, c.pos));
                        continue;
                    }
                }
                lex_number(&mut c, src, start, &mut out)?;
            }
            _ => {
                let ch = src[start..].chars().next().unwrap();
                if is_ident_start(ch) {
                    // `infix[l|r]? <digit> <op>` is a Fixity token.
                    if let Some(tok) = try_fixity(&mut c, src, start)? {
                        out.push((start, tok, c.pos));
                        continue;
                    }
                    let mut end = start + ch.len_utf8();
                    while src[end..].chars().next().is_some_and(is_ident_char) {
                        end += src[end..].chars().next().unwrap().len_utf8();
                    }
                    let word = &src[start..end];
                    c.bump(end - start);
                    if ["import", "module", "where"].contains(&word) {
                        return Err(LexError {
                            offset: start,
                            message: format!("{word} is a reserved keyword"),
                        });
                    }
                    if let Some(tok) = keyword(word) {
                        out.push((start, tok, c.pos));
                    } else {
                        out.push((start, Tok::Id(word.to_string()), c.pos));
                    }
                } else if is_oper_char(ch) {
                    lex_operator(&mut c, src, start, &mut out)?;
                } else {
                    return Err(c.err(format!("unexpected character: {ch}")));
                }
            }
        }
    }
    Ok(out)
}

fn lex_operator(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    // Comment openings are handled before this function; `/` elsewhere is
    // an operator character, including as a standalone division operator.
    // A trailing `_ident` suffix belongs to the operator (ocamllex `operatorn`).
    let mut end = c.pos;
    while c.src.get(end).is_some_and(|&x| is_oper_char(x as char)) {
        end += 1;
    }
    if end == c.pos {
        return Err(c.err("expected operator"));
    }
    if c.src.get(end) == Some(&b'_') {
        let mut k = end + 1;
        let rest = &src[k..];
        let mut chars = rest.chars();
        if chars.next().is_some_and(is_ident_start) {
            k += rest.chars().next().unwrap().len_utf8();
            while src[k..].chars().next().is_some_and(is_ident_char) {
                k += src[k..].chars().next().unwrap().len_utf8();
            }
            end = k;
        }
    }
    let op = &src[c.pos..end];
    let token = match op {
        "@" => Tok::At, "^" => Tok::Caret, "::" => Tok::ColonColon,
        ":" => Tok::Colon, ".." => Tok::DotDot, "." => Tok::Dot,
        "=" => Tok::Eq, "*" => Tok::Star, "|" => Tok::Bar,
        "=>" => Tok::EqGt, "->" => Tok::MinusGt, "<->" => Tok::Bidir,
        "-" => Tok::Minus,
        _ => Tok::OpId(op.to_string()),
    };
    c.bump(end - c.pos);
    out.push((start, token, c.pos));
    Ok(())
}

fn try_fixity(c: &mut Cursor<'_>, src: &str, start: usize) -> Result<Option<Tok>, LexError> {
    let rest = &src[start..];
    let (kw, assoc) = if rest.starts_with("infixl") && rest["infixl".len()..].starts_with([' ', '\t']) {
        ("infixl", FixAssoc::InfixL)
    } else if rest.starts_with("infixr") && rest["infixr".len()..].starts_with([' ', '\t']) {
        ("infixr", FixAssoc::InfixR)
    } else if rest.starts_with("infix") && rest["infix".len()..].starts_with([' ', '\t']) {
        ("infix", FixAssoc::Infix)
    } else {
        return Ok(None);
    };
    let mut k = start + kw.len();
    while src[k..].starts_with([' ', '\t']) {
        k += 1;
    }
    let digit = src[k..].chars().next().unwrap_or('?');
    if !digit.is_ascii_digit() {
        return Ok(None);
    }
    let prec = digit.to_digit(10).unwrap() as u8;
    k += 1;
    if !src[k..].starts_with([' ', '\t']) {
        return Ok(None);
    }
    while src[k..].starts_with([' ', '\t']) {
        k += 1;
    }
    let mut op_end = k;
    while src[op_end..].chars().next().is_some_and(is_oper_char) {
        op_end += src[op_end..].chars().next().unwrap().len_utf8();
    }
    if op_end == k {
        return Ok(None);
    }
    let op = src[k..op_end].to_string();
    c.bump(op_end - start);
    Ok(Some(Tok::Fixity(assoc, prec, op)))
}

fn lex_number(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    let rest = &src[start..];
    if rest.starts_with("0b") && rest[2..].chars().next().is_some_and(|x| x == '0' || x == '1' || x == '_') {
        let mut end = start + 2;
        while src[end..].chars().next().is_some_and(|x| x == '0' || x == '1' || x == '_') {
            end += 1;
        }
        let s = src[start + 2..end].to_string();
        c.bump(end - start);
        out.push((start, Tok::Bin(s), c.pos));
        return Ok(());
    }
    if rest.starts_with("0x")
        && rest[2..].chars().next().is_some_and(|x| x.is_ascii_hexdigit() || x == '_')
    {
        let mut end = start + 2;
        while src[end..].chars().next().is_some_and(|x| x.is_ascii_hexdigit() || x == '_') {
            end += 1;
        }
        let s = src[start + 2..end].to_string();
        c.bump(end - start);
        out.push((start, Tok::Hex(s), c.pos));
        return Ok(());
    }
    let mut end = start;
    while src[end..].chars().next().is_some_and(|x| x.is_ascii_digit()) {
        end += 1;
    }
    // `12.34` is a Real (like ocamllex `(digit+) "." (digit+)`).
    if src[end..].starts_with('.') && src[end + 1..].chars().next().is_some_and(|x| x.is_ascii_digit()) {
        end += 1;
        while src[end..].chars().next().is_some_and(|x| x.is_ascii_digit()) {
            end += 1;
        }
        let s = src[start..end].to_string();
        c.bump(end - start);
        out.push((start, Tok::Real(s), c.pos));
        return Ok(());
    }
    let s = src[start..end].to_string();
    c.bump(end - start);
    out.push((start, Tok::Num(s), c.pos));
    Ok(())
}

fn lex_string(c: &mut Cursor<'_>) -> Result<String, LexError> {
    // opening `"` already peeked
    c.bump(1);
    let mut s = String::new();
    loop {
        let Some(b) = c.peek() else {
            return Err(c.err("file ended before closing quote in string"));
        };
        match b {
            b'"' => {
                c.bump(1);
                return Ok(s);
            }
            b'\n' => return Err(c.err("newline in string literal")),
            b'\\' => {
                c.bump(1);
                match c.peek() {
                    Some(b'n') => s.push('\n'),
                    Some(b't') => s.push('\t'),
                    Some(b'r') => s.push('\r'),
                    Some(b'b') => s.push('\x08'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'"') => s.push('"'),
                    Some(b'\'') => s.push('\''),
                    Some(x) => {
                        s.push('\\');
                        s.push(x as char);
                    }
                    None => return Err(c.err("file ended in string escape")),
                }
                c.bump(1);
            }
            _ => {
                let ch = c.src[c.pos..].to_vec();
                let ch = std::str::from_utf8(&ch).map_err(|_| c.err("invalid utf-8"))?;
                let ch = ch.chars().next().unwrap();
                s.push(ch);
                c.bump(ch.len_utf8());
            }
        }
    }
}

fn lex_multiline_string(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    // `"""` already peeked; consume up to the end of the line, then lines
    // until a line containing only whitespace + `"""` (joined with `\n`).
    c.bump(3);
    while matches!(c.peek(), Some(b' ' | b'\t')) {
        c.bump(1);
    }
    if c.peek() != Some(b'\n') {
        return Err(c.err("expected newline after \"\"\""));
    }
    c.bump(1);
    let mut lines: Vec<&str> = Vec::new();
    loop {
        let line_start = c.pos;
        while !matches!(c.peek(), None | Some(b'\n')) {
            c.bump(1);
        }
        if c.peek().is_none() {
            return Err(c.err("file ended before closing \"\"\""));
        }
        let line = &src[line_start..c.pos];
        c.bump(1); // newline
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.starts_with("\"\"\"") {
            let indent = &line[..line.len() - trimmed.len()];
            let s = lines
                .iter()
                .map(|l| l.strip_prefix(indent).unwrap_or(l))
                .collect::<Vec<_>>()
                .join("\n");
            c.pos = line_start + (line.len() - trimmed.len()) + 3;
            out.push((start, Tok::Str(s), c.pos));
            return Ok(());
        }
        lines.push(line);
    }
}

fn lex_doc_line(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    // `///` peeked
    c.bump(3);
    let mut text = String::new();
    loop {
        let line_start = c.pos;
        while !matches!(c.peek(), None | Some(b'\n')) {
            c.bump(1);
        }
        text.push_str(&src[line_start..c.pos]);
        if c.peek().is_none() {
            break;
        }
        c.bump(1); // newline
        // continuation: whitespace* `///`
        let mut k = c.pos;
        while src[k..].starts_with([' ', '\t']) {
            k += 1;
        }
        if src[k..].starts_with("///") {
            text.push('\n');
            c.pos = k + 3;
        } else {
            break;
        }
    }
    out.push((start, Tok::DocLine(text), c.pos));
    Ok(())
}

fn lex_doc_block(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    // `/*!` peeked; nesting allowed
    c.bump(3);
    let mut depth = 0usize;
    let mut text = String::from("/*!");
    loop {
        let Some(_) = c.peek() else {
            return Err(c.err("file ended before end of doc comment"));
        };
        if c.starts_with("/*!") || c.starts_with("/*") {
            depth += 1;
            text.push_str(&src[c.pos..c.pos + 2]);
            c.bump(2);
        } else if c.starts_with("*/") {
            if depth == 0 {
                c.bump(2);
                out.push((start, Tok::DocBlock(text), c.pos));
                return Ok(());
            }
            depth -= 1;
            text.push_str("*/");
            c.bump(2);
        } else {
            let ch = src[c.pos..].chars().next().unwrap();
            text.push(ch);
            c.bump(ch.len_utf8());
        }
    }
}

fn skip_block_comment(c: &mut Cursor<'_>) -> Result<(), LexError> {
    // `/*` peeked; nesting allowed
    c.bump(2);
    let mut depth = 0usize;
    loop {
        let Some(_) = c.peek() else {
            return Err(c.err("file ended before end of comment"));
        };
        if c.starts_with("/*") {
            depth += 1;
            c.bump(2);
        } else if c.starts_with("*/") {
            if depth == 0 {
                c.bump(2);
                return Ok(());
            }
            depth -= 1;
            c.bump(2);
        } else {
            c.bump(1);
        }
    }
}

fn is_directive_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '#' | '?')
}

fn lex_directive(c: &mut Cursor<'_>, src: &str, start: usize, out: &mut Vec<(usize, Tok, usize)>) -> Result<(), LexError> {
    // `$` peeked.
    c.bump(1);
    if c.peek() == Some(b'[') {
        // `$[name` → attribute token; the rest tokenizes normally.
        c.bump(1);
        let name = lex_directive_name(c, src)?;
        out.push((start, Tok::Attr(name), c.pos));
        return Ok(());
    }
    let name = lex_directive_name(c, src)?;
    // `$name {` → structured pragma (the `{` belongs to the token).
    let mut k = c.pos;
    while src[k..].starts_with([' ', '\t']) {
        k += 1;
    }
    if src[k..].starts_with('{') {
        c.pos = k + 1;
        out.push((start, Tok::StructPragma(name), c.pos));
        return Ok(());
    }
    // `$name args...` → line pragma, argument runs to end of line.
    // (Block comments inside pragma arguments are not supported here;
    // `//` starts a comment like in the OCaml lexer.)
    let mut arg_end = c.pos;
    while !matches!(src[arg_end..].chars().next(), None | Some('\n')) {
        if src[arg_end..].starts_with("//") {
            break;
        }
        arg_end += src[arg_end..].chars().next().unwrap().len_utf8();
    }
    let arg = src[c.pos..arg_end].trim().to_string();
    c.pos = arg_end;
    out.push((start, Tok::Pragma(name, arg), c.pos));
    Ok(())
}

fn lex_directive_name(c: &mut Cursor<'_>, src: &str) -> Result<String, LexError> {
    let s = c.pos;
    let mut end = s;
    while src[end..].chars().next().is_some_and(is_directive_char) {
        end += src[end..].chars().next().unwrap().len_utf8();
    }
    if end == s {
        return Err(c.err("expected directive name after `$`"));
    }
    let name = src[s..end].to_string();
    c.bump(end - s);
    Ok(name)
}
