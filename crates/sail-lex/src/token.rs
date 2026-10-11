#![forbid(unsafe_code)]
//! Token stream types for the Sail2 lexer.
//!
//! The inventory mirrors `sail/src/lib/token.ml` (OCaml reference): every
//! keyword, literal, operator and piece of punctuation the parser lane needs.
//! Payload conventions (`Hex`/`Bin` without prefix, `Num` as decimal text)
//! match the OCaml lexer; see `ARCHITECTURE.md` for the swap contract.
//!
//! Payloads are interned [`Symbol`]s (Lane 71 payload-intern):
//! [`lex`][crate::lex] takes
//! the caller's interner, so repeated identifiers/literals share one table
//! entry instead of one `String` per occurrence. Resolve a payload with
//! [`Interner::resolve`]. The only `String` payload left is
//! [`TokenKind::Error`] (recovery messages duplicate [`LexError`]`::message`
//! and must not pollute the table).

use crate::Span;
use sail_ast::{Interner, Symbol};

/// `infix`/`infixl`/`infixr` associativity attached to [`TokenKind::Fixity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fixity {
    Infix,
    InfixL,
    InfixR,
}

/// One lexical token kind. Keywords are unit variants; identifiers, literals,
/// operators and directives carry interned [`Symbol`] payloads (resolve via
/// the [`lex`][crate::lex] caller's interner).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    // -- identifiers, type variables, operators -----------------------------
    /// Plain identifier, incl. the special `~` (as in the OCaml lexer).
    Id(Symbol),
    /// Type-level variable incl. leading `'`, e.g. `'a`.
    TyVar(Symbol),
    /// Symbolic operator, e.g. `++`, `>=`.
    Op(Symbol),

    // -- literals ------------------------------------------------------------
    /// Decimal integer literal, e.g. `"42"` (arbitrary width, kept as text).
    Num(Symbol),
    /// Hex digits without `0x` prefix, e.g. `"FF"` (`_` preserved as written).
    Hex(Symbol),
    /// Binary digits without `0b` prefix, e.g. `"101"`.
    Bin(Symbol),
    /// Real literal in decimal notation, e.g. `"1.5"`.
    Real(Symbol),
    /// String literal, escape sequences resolved.
    Str(Symbol),
    /// `"""` multi-line string, indentation prefix stripped, lines joined.
    MultiLineStr(Symbol),

    // -- keywords ------------------------------------------------------------
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
    OpKw,
    Default,
    Effect,
    End,
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
    /// `Int` kind keyword.
    KindInt,
    /// `Nat` kind keyword.
    KindNat,
    /// `Order` kind keyword.
    KindOrder,
    /// `Bool` kind keyword.
    KindBool,
    /// `Type` kind keyword.
    KindType,
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
    /// `undefined` literal keyword.
    Undefined,
    /// `type` definition keyword.
    Typedef,
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

    // -- punctuation ---------------------------------------------------------
    At,
    /// `2^` (with optional whitespace between, as in the OCaml lexer).
    TwoCaret,
    Caret,
    ColonColon,
    Colon,
    Comma,
    DotDot,
    Dot,
    Eq,
    Semi,
    Star,
    Under,
    LsquareBar,
    RsquareBar,
    LcurlyBar,
    RcurlyBar,
    Bar,
    Lcurly,
    Rcurly,
    /// `()`.
    Unit,
    Lparen,
    Rparen,
    Lsquare,
    Rsquare,
    MinusGt,
    Minus,
    Bidir,
    EqGt,

    // -- directives, attributes, docs ----------------------------------------
    /// `$[name` attribute opener (rest lexes as normal tokens).
    Attribute(Symbol),
    /// `$name {` structured pragma opener.
    StructuredPragma(Symbol),
    /// `$name value…` up to end of line.
    Pragma {
        name: Symbol,
        value: Symbol,
    },
    /// `infix[l|r] N op` fixity declaration.
    Fixity {
        fixity: Fixity,
        level: u8,
        op: Symbol,
    },
    /// `/*! … */` documentation block.
    DocBlock(Symbol),
    /// `/// …` documentation line(s).
    DocLine(Symbol),

    /// End of input (always the last token).
    Eof,
    /// Recovery token: the lexer skipped bad input and continued.
    /// The message duplicates the matching [`crate::LexError`] and stays a
    /// `String` on purpose (error text must not pollute the interner table).
    Error(String),
}

/// A token with its source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    #[must_use]
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }

    #[must_use]
    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }

    /// Documentation tokens a parser usually skips (`DocBlock`/`DocLine`).
    #[must_use]
    pub fn is_doc(&self) -> bool {
        matches!(self.kind, TokenKind::DocBlock(_) | TokenKind::DocLine(_))
    }

    /// Short stable name for debugging/snapshots (no payload).
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        self.kind.name()
    }
}

impl TokenKind {
    /// Short stable name for debugging/snapshots (no payload).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            TokenKind::Id(_) => "Id",
            TokenKind::TyVar(_) => "TyVar",
            TokenKind::Op(_) => "Op",
            TokenKind::Num(_) => "Num",
            TokenKind::Hex(_) => "Hex",
            TokenKind::Bin(_) => "Bin",
            TokenKind::Real(_) => "Real",
            TokenKind::Str(_) => "Str",
            TokenKind::MultiLineStr(_) => "MultiLineStr",
            TokenKind::And => "and",
            TokenKind::As => "as",
            TokenKind::Assert => "assert",
            TokenKind::Bitzero => "bitzero",
            TokenKind::Bitone => "bitone",
            TokenKind::By => "by",
            TokenKind::Match => "match",
            TokenKind::Config => "config",
            TokenKind::Clause => "clause",
            TokenKind::Dec => "dec",
            TokenKind::OpKw => "operator",
            TokenKind::Default => "default",
            TokenKind::Effect => "effect",
            TokenKind::End => "end",
            TokenKind::Enum => "enum",
            TokenKind::Else => "else",
            TokenKind::Exit => "exit",
            TokenKind::Cast => "cast",
            TokenKind::False => "false",
            TokenKind::Forall => "forall",
            TokenKind::Foreach => "foreach",
            TokenKind::Function => "function",
            TokenKind::Mapping => "mapping",
            TokenKind::Overload => "overload",
            TokenKind::Throw => "throw",
            TokenKind::Try => "try",
            TokenKind::Catch => "catch",
            TokenKind::If => "if",
            TokenKind::In => "in",
            TokenKind::Inc => "inc",
            TokenKind::Let => "let",
            TokenKind::Var => "var",
            TokenKind::Ref => "ref",
            TokenKind::KindInt => "Int",
            TokenKind::KindNat => "Nat",
            TokenKind::KindOrder => "Order",
            TokenKind::KindBool => "Bool",
            TokenKind::KindType => "Type",
            TokenKind::Pure => "pure",
            TokenKind::Impure => "impure",
            TokenKind::Monadic => "monadic",
            TokenKind::Register => "register",
            TokenKind::Return => "return",
            TokenKind::Scattered => "scattered",
            TokenKind::Sizeof => "sizeof",
            TokenKind::Constraint => "constraint",
            TokenKind::Constant => "constant",
            TokenKind::Struct => "struct",
            TokenKind::Then => "then",
            TokenKind::True => "true",
            TokenKind::Undefined => "undefined",
            TokenKind::Typedef => "type",
            TokenKind::Union => "union",
            TokenKind::Newtype => "newtype",
            TokenKind::With => "with",
            TokenKind::When => "when",
            TokenKind::Val => "val",
            TokenKind::Outcome => "outcome",
            TokenKind::Instantiation => "instantiation",
            TokenKind::Impl => "impl",
            TokenKind::Private => "private",
            TokenKind::Repeat => "repeat",
            TokenKind::Until => "until",
            TokenKind::While => "while",
            TokenKind::Do => "do",
            TokenKind::Mutual => "mutual",
            TokenKind::Bitfield => "bitfield",
            TokenKind::Configuration => "configuration",
            TokenKind::TerminationMeasure => "termination_measure",
            TokenKind::Forwards => "forwards",
            TokenKind::Backwards => "backwards",
            TokenKind::From => "from",
            TokenKind::To => "to",
            TokenKind::Downto => "downto",
            TokenKind::InternalPLet => "internal_plet",
            TokenKind::InternalReturn => "internal_return",
            TokenKind::InternalAssume => "internal_assume",
            TokenKind::At => "@",
            TokenKind::TwoCaret => "2^",
            TokenKind::Caret => "^",
            TokenKind::ColonColon => "::",
            TokenKind::Colon => ":",
            TokenKind::Comma => ",",
            TokenKind::DotDot => "..",
            TokenKind::Dot => ".",
            TokenKind::Eq => "=",
            TokenKind::Semi => ";",
            TokenKind::Star => "*",
            TokenKind::Under => "_",
            TokenKind::LsquareBar => "[|",
            TokenKind::RsquareBar => "|]",
            TokenKind::LcurlyBar => "{|",
            TokenKind::RcurlyBar => "|}",
            TokenKind::Bar => "|",
            TokenKind::Lcurly => "{",
            TokenKind::Rcurly => "}",
            TokenKind::Unit => "()",
            TokenKind::Lparen => "(",
            TokenKind::Rparen => ")",
            TokenKind::Lsquare => "[",
            TokenKind::Rsquare => "]",
            TokenKind::MinusGt => "->",
            TokenKind::Minus => "-",
            TokenKind::Bidir => "<->",
            TokenKind::EqGt => "=>",
            TokenKind::Attribute(_) => "Attribute",
            TokenKind::StructuredPragma(_) => "StructuredPragma",
            TokenKind::Pragma { .. } => "Pragma",
            TokenKind::Fixity { .. } => "Fixity",
            TokenKind::DocBlock(_) => "DocBlock",
            TokenKind::DocLine(_) => "DocLine",
            TokenKind::Eof => "Eof",
            TokenKind::Error(_) => "Error",
        }
    }

    /// Look up a keyword by its source text.
    #[must_use]
    pub fn keyword(word: &str) -> Option<TokenKind> {
        Some(match word {
            "and" => TokenKind::And,
            "as" => TokenKind::As,
            "assert" => TokenKind::Assert,
            "bitzero" => TokenKind::Bitzero,
            "bitone" => TokenKind::Bitone,
            "by" => TokenKind::By,
            "match" => TokenKind::Match,
            "config" => TokenKind::Config,
            "clause" => TokenKind::Clause,
            "dec" => TokenKind::Dec,
            "operator" => TokenKind::OpKw,
            "default" => TokenKind::Default,
            "effect" => TokenKind::Effect,
            "end" => TokenKind::End,
            "enum" => TokenKind::Enum,
            "else" => TokenKind::Else,
            "exit" => TokenKind::Exit,
            "cast" => TokenKind::Cast,
            "false" => TokenKind::False,
            "forall" => TokenKind::Forall,
            "foreach" => TokenKind::Foreach,
            "function" => TokenKind::Function,
            "mapping" => TokenKind::Mapping,
            "overload" => TokenKind::Overload,
            "throw" => TokenKind::Throw,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "if" => TokenKind::If,
            "in" => TokenKind::In,
            "inc" => TokenKind::Inc,
            "let" => TokenKind::Let,
            "var" => TokenKind::Var,
            "ref" => TokenKind::Ref,
            "Int" => TokenKind::KindInt,
            "Nat" => TokenKind::KindNat,
            "Order" => TokenKind::KindOrder,
            "Bool" => TokenKind::KindBool,
            "pure" => TokenKind::Pure,
            "impure" => TokenKind::Impure,
            "monadic" => TokenKind::Monadic,
            "register" => TokenKind::Register,
            "return" => TokenKind::Return,
            "scattered" => TokenKind::Scattered,
            "sizeof" => TokenKind::Sizeof,
            "constraint" => TokenKind::Constraint,
            "constant" => TokenKind::Constant,
            "struct" => TokenKind::Struct,
            "then" => TokenKind::Then,
            "true" => TokenKind::True,
            "undefined" => TokenKind::Undefined,
            "Type" => TokenKind::KindType,
            "type" => TokenKind::Typedef,
            "union" => TokenKind::Union,
            "newtype" => TokenKind::Newtype,
            "with" => TokenKind::With,
            "when" => TokenKind::When,
            "val" => TokenKind::Val,
            "outcome" => TokenKind::Outcome,
            "instantiation" => TokenKind::Instantiation,
            "impl" => TokenKind::Impl,
            "private" => TokenKind::Private,
            "repeat" => TokenKind::Repeat,
            "until" => TokenKind::Until,
            "while" => TokenKind::While,
            "do" => TokenKind::Do,
            "mutual" => TokenKind::Mutual,
            "bitfield" => TokenKind::Bitfield,
            "configuration" => TokenKind::Configuration,
            "termination_measure" => TokenKind::TerminationMeasure,
            "forwards" => TokenKind::Forwards,
            "backwards" => TokenKind::Backwards,
            "from" => TokenKind::From,
            "to" => TokenKind::To,
            "downto" => TokenKind::Downto,
            "internal_plet" => TokenKind::InternalPLet,
            "internal_return" => TokenKind::InternalReturn,
            "internal_assume" => TokenKind::InternalAssume,
            _ => return None,
        })
    }
}
