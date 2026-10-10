#![forbid(unsafe_code)]
//! Behaviour tests for the lexer, pinned against `lexer.mll`.
//!
//! Lane 71: payloads are interned [`Symbol`]s. [`kinds`] returns the token
//! kinds plus the table they were interned into, so expectations intern
//! through the same table (identical symbols, no string comparison).

use sail_ast::{Interner as _, SimpleInterner, Symbol};
use sail_lex::{lex, FileId, Fixity, LexError, LexResult, Span, Token, TokenKind};

/// Lex `src` with a fresh interner; the returned table resolves every
/// payload symbol in the returned kinds.
fn kinds(src: &str) -> (Vec<TokenKind>, SimpleInterner) {
    let mut it = SimpleInterner::new();
    let out = lex(FileId(0), src, &mut it);
    let kinds = out.tokens.into_iter().map(|t| t.kind).collect();
    (kinds, it)
}

/// Lex `src` for `file` with a fresh interner (full result + table).
fn lexed(file: FileId, src: &str) -> (LexResult, SimpleInterner) {
    let mut it = SimpleInterner::new();
    let out = lex(file, src, &mut it);
    (out, it)
}

fn sym(it: &mut SimpleInterner, s: &str) -> Symbol {
    it.intern(s)
}

fn id(it: &mut SimpleInterner, s: &str) -> TokenKind {
    TokenKind::Id(sym(it, s))
}

fn op(it: &mut SimpleInterner, s: &str) -> TokenKind {
    TokenKind::Op(sym(it, s))
}

#[test]
fn basic_function_shape_and_spans() {
    let (out, _it) = lexed(FileId(7), "function foo(x) = { x }");
    assert!(out.is_ok());
    let names: Vec<&str> = out.tokens.iter().map(|t| t.kind_name()).collect();
    assert_eq!(
        names,
        ["function", "Id", "(", "Id", ")", "=", "{", "Id", "}", "Eof"]
    );
    assert_eq!(out.tokens[0].span, Span::new(FileId(7), 0, 8));
    assert_eq!(out.tokens[1].span, Span::new(FileId(7), 9, 12));
    assert!(out.tokens.iter().all(|t| t.span.file == FileId(7)));
}

#[test]
fn keywords_vs_identifiers() {
    let (got, mut it) = kinds("match matchy Type Typey true truthy undefined undef");
    assert_eq!(
        got,
        [
            TokenKind::Match,
            id(&mut it, "matchy"),
            TokenKind::KindType,
            id(&mut it, "Typey"),
            TokenKind::True,
            id(&mut it, "truthy"),
            TokenKind::Undefined,
            id(&mut it, "undef"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn tilde_is_plain_id_like_ocaml() {
    let (got, mut it) = kinds("~");
    assert_eq!(got, [id(&mut it, "~"), TokenKind::Eof]);
}

#[test]
fn underscore_and_unit() {
    let (got, mut it) = kinds("_ _x () ( )");
    assert_eq!(
        got,
        [
            TokenKind::Under,
            id(&mut it, "_x"),
            TokenKind::Unit,
            TokenKind::Lparen,
            TokenKind::Rparen,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn tyvar_and_lone_quote_error() {
    let (got, mut it) = kinds("'a 'foo");
    assert_eq!(
        got,
        [
            TokenKind::TyVar(sym(&mut it, "'a")),
            TokenKind::TyVar(sym(&mut it, "'foo")),
            TokenKind::Eof,
        ]
    );
    let (out, _it) = lexed(FileId(0), "a ' b");
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains('\''));
    let (got, mut it) = kinds("a ' b");
    assert_eq!(
        got,
        [
            id(&mut it, "a"),
            TokenKind::Error("Unexpected character: `'`".to_string()),
            id(&mut it, "b"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn numbers_reals_ranges() {
    let (got, mut it) = kinds("42 1.5 1..2 1. 0");
    assert_eq!(
        got,
        [
            TokenKind::Num(sym(&mut it, "42")),
            TokenKind::Real(sym(&mut it, "1.5")),
            TokenKind::Num(sym(&mut it, "1")),
            TokenKind::DotDot,
            TokenKind::Num(sym(&mut it, "2")),
            TokenKind::Num(sym(&mut it, "1")),
            TokenKind::Dot,
            TokenKind::Num(sym(&mut it, "0")),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn bit_literals_hex_bin() {
    let (out, _it) = lexed(FileId(0), "0xFF 0b101 0xFFFF_0000_1234_0000 0b1_0");
    assert!(out.is_ok());
    let (got, mut it) = kinds("0xFF 0b101 0xFFFF_0000_1234_0000 0b1_0");
    assert_eq!(
        got,
        [
            TokenKind::Hex(sym(&mut it, "FF")),
            TokenKind::Bin(sym(&mut it, "101")),
            TokenKind::Hex(sym(&mut it, "FFFF_0000_1234_0000")),
            TokenKind::Bin(sym(&mut it, "1_0")),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn bare_0x_falls_back_to_num_plus_id() {
    // Mirrors ocamllex: `0x` needs hex digits, otherwise `Num("0") Id(...)`.
    let (got, mut it) = kinds("0x + 1");
    assert_eq!(
        got,
        [
            TokenKind::Num(sym(&mut it, "0")),
            id(&mut it, "x"),
            op(&mut it, "+"),
            TokenKind::Num(sym(&mut it, "1")),
            TokenKind::Eof,
        ]
    );
    let (got, mut it) = kinds("0b2");
    assert_eq!(
        got,
        [
            TokenKind::Num(sym(&mut it, "0")),
            id(&mut it, "b2"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn two_caret_forms() {
    let (got, mut it) = kinds("2^3");
    assert_eq!(
        got,
        [
            TokenKind::TwoCaret,
            TokenKind::Num(sym(&mut it, "3")),
            TokenKind::Eof
        ]
    );
    let (got, mut it) = kinds("2 ^ 3");
    assert_eq!(
        got,
        [
            TokenKind::TwoCaret,
            TokenKind::Num(sym(&mut it, "3")),
            TokenKind::Eof
        ]
    );
    // Longer digit runs are `Num` + `Caret`.
    let (got, mut it) = kinds("12^3");
    assert_eq!(
        got,
        [
            TokenKind::Num(sym(&mut it, "12")),
            TokenKind::Caret,
            TokenKind::Num(sym(&mut it, "3")),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn comments_are_skipped_and_nest() {
    let (out, _it) = lexed(FileId(0), "a /* x /* y */ z */ b // tail\nc");
    assert!(out.is_ok());
    assert_eq!(out.tokens.iter().filter(|t| !t.is_eof()).count(), 3);
    let (got, mut it) = kinds("a /* x /* y */ z */ b // tail\nc");
    assert_eq!(
        got,
        [
            id(&mut it, "a"),
            id(&mut it, "b"),
            id(&mut it, "c"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn paren_star_comment_needs_whitespace() {
    // Lane-spec `(* *)` comments …
    let (got, mut it) = kinds("a (* hello *) b");
    assert_eq!(got, [id(&mut it, "a"), id(&mut it, "b"), TokenKind::Eof]);
    let (got, mut it) = kinds("a (*\nhello\n*) b");
    assert_eq!(got, [id(&mut it, "a"), id(&mut it, "b"), TokenKind::Eof]);
    // … while real code like `(*TVAL)` still lexes as `(` `*`.
    let (got, mut it) = kinds("(*TVAL) = x");
    assert_eq!(
        got,
        [
            TokenKind::Lparen,
            TokenKind::Star,
            id(&mut it, "TVAL"),
            TokenKind::Rparen,
            TokenKind::Eq,
            id(&mut it, "x"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn unbalanced_comment_is_recoverable() {
    let (out, _it) = lexed(FileId(0), "a /* oops");
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains("Unbalanced"));
    assert!(out.tokens.last().is_some_and(|t| t.is_eof()));
}

#[test]
fn stray_comment_closer_is_an_error() {
    let (out, _it) = lexed(FileId(0), "a */ b");
    assert_eq!(out.errors.len(), 1);
    let (got, mut it) = kinds("a */ b");
    assert_eq!(
        got,
        [
            id(&mut it, "a"),
            TokenKind::Error("Unbalanced comment: stray `*/`".to_string()),
            id(&mut it, "b"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn strings_with_escapes() {
    let (out, _it) = lexed(FileId(0), r#""a\nb\"q\"\\A\x41\065""#);
    assert!(out.is_ok(), "errors: {:?}", out.errors);
    let (got, mut it) = kinds(r#""a\nb\"q\"\\A\x41\065""#);
    assert_eq!(
        got,
        [
            TokenKind::Str(sym(&mut it, "a\nb\"q\"\\AAA")),
            TokenKind::Eof
        ]
    );
}

#[test]
fn string_continuation_and_raw_newline() {
    let (out, _it) = lexed(FileId(0), "\"a\\\n   b\"");
    assert!(out.is_ok());
    let (got, mut it) = kinds("\"a\\\n   b\"");
    assert_eq!(got, [TokenKind::Str(sym(&mut it, "ab")), TokenKind::Eof]);

    let (out, _it) = lexed(FileId(0), "\"a\nb\"");
    assert!(out.is_ok());
    let (got, mut it) = kinds("\"a\nb\"");
    assert_eq!(got, [TokenKind::Str(sym(&mut it, "a\nb")), TokenKind::Eof]);
}

#[test]
fn unterminated_string_recovers() {
    let (out, mut it) = lexed(FileId(0), "\"abc");
    assert_eq!(out.errors.len(), 1);
    assert_eq!(out.tokens[0].kind, TokenKind::Str(sym(&mut it, "abc")),);
    assert!(out.tokens.last().is_some_and(|t| t.is_eof()));
}

#[test]
fn illegal_escape_recovers_inline() {
    let (out, mut it) = lexed(FileId(0), "\"a\\qb\"");
    assert_eq!(out.errors.len(), 1);
    // Policy: the offending span leaves an `Error` token behind; the string
    // itself continues and is still emitted.
    assert!(matches!(out.tokens[0].kind, TokenKind::Error(_)));
    assert_eq!(out.tokens[1].kind, TokenKind::Str(sym(&mut it, "aqb")));
}

#[test]
fn multiline_string_strips_prefix() {
    let src = "\"\"\"\n  hello\n\n  world\n  \"\"\"";
    let (out, mut it) = lexed(FileId(0), src);
    assert!(out.is_ok(), "errors: {:?}", out.errors);
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::MultiLineStr(sym(&mut it, "hello\n\nworld"))
    );
}

#[test]
fn multiline_string_bad_prefix_is_an_error() {
    let src = "\"\"\"\n  hello\nnoprefix\n  \"\"\"";
    let (out, _it) = lexed(FileId(0), src);
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains("prefix"));
}

#[test]
fn operators_and_arrows() {
    let (got, mut it) = kinds("x == y => z -> w <-> v ==> u");
    assert_eq!(
        got,
        [
            id(&mut it, "x"),
            op(&mut it, "=="),
            id(&mut it, "y"),
            TokenKind::EqGt,
            id(&mut it, "z"),
            TokenKind::MinusGt,
            id(&mut it, "w"),
            TokenKind::Bidir,
            id(&mut it, "v"),
            op(&mut it, "==>"),
            id(&mut it, "u"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn long_arrow_becomes_operator() {
    // Longest match wins, as in ocamllex: `a-->b` is `Id Op("-->") Id`.
    let (got, mut it) = kinds("a-->b");
    assert_eq!(
        got,
        [
            id(&mut it, "a"),
            op(&mut it, "-->"),
            id(&mut it, "b"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn slash_before_comment_ends_operator() {
    let (got, mut it) = kinds("+/*c*/b //x\n");
    assert_eq!(got, [op(&mut it, "+"), id(&mut it, "b"), TokenKind::Eof]);
}

#[test]
fn colons_dots_pipes() {
    let (got, mut it) = kinds(":: ::: := : .. ... . | || [| |] {| |}");
    assert_eq!(
        got,
        [
            TokenKind::ColonColon,
            op(&mut it, ":::"),
            op(&mut it, ":="),
            TokenKind::Colon,
            TokenKind::DotDot,
            op(&mut it, "..."),
            TokenKind::Dot,
            TokenKind::Bar,
            op(&mut it, "||"),
            TokenKind::LsquareBar,
            TokenKind::RsquareBar,
            TokenKind::LcurlyBar,
            TokenKind::RcurlyBar,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn fixity_declarations() {
    let (got, mut it) = kinds("infixl 8 ++");
    assert_eq!(
        got,
        [
            TokenKind::Fixity {
                fixity: Fixity::InfixL,
                level: 8,
                op: sym(&mut it, "++"),
            },
            TokenKind::Eof,
        ]
    );
    let (got, mut it) = kinds("infix 4 <->");
    assert_eq!(
        got,
        [
            TokenKind::Fixity {
                fixity: Fixity::Infix,
                level: 4,
                op: sym(&mut it, "<->"),
            },
            TokenKind::Eof,
        ]
    );
    // Without `N op` these are plain identifiers.
    let (got, mut it) = kinds("infix infixl 8 infixr");
    assert_eq!(
        got,
        [
            id(&mut it, "infix"),
            id(&mut it, "infixl"),
            TokenKind::Num(sym(&mut it, "8")),
            id(&mut it, "infixr"),
            TokenKind::Eof
        ]
    );
}

#[test]
fn directives_pragmas_attributes() {
    let (out, mut it) = lexed(FileId(0), "$include <prelude.sail>\n");
    assert!(out.is_ok());
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::Pragma {
            name: sym(&mut it, "include"),
            value: sym(&mut it, " <prelude.sail>"),
        }
    );
    let (got, mut it) = kinds("$foo {");
    assert_eq!(
        got,
        [
            TokenKind::StructuredPragma(sym(&mut it, "foo")),
            TokenKind::Eof
        ]
    );
    let (got, mut it) = kinds("$[mycfg]");
    assert_eq!(
        got,
        [
            TokenKind::Attribute(sym(&mut it, "mycfg")),
            TokenKind::Rsquare,
            TokenKind::Eof,
        ]
    );
    let (out, _it) = lexed(FileId(0), "$\n");
    assert_eq!(out.errors.len(), 1);
}

#[test]
fn reserved_words_are_errors_not_idents() {
    for word in ["import", "module", "where"] {
        let (out, _it) = lexed(FileId(0), word);
        assert_eq!(out.errors.len(), 1, "{word}");
        assert!(out.errors[0].message.contains("reserved"), "{word}");
        assert!(matches!(out.tokens[0].kind, TokenKind::Error(_)), "{word}");
    }
}

#[test]
fn unexpected_characters_recover() {
    let (out, _it) = lexed(FileId(0), "a ` b");
    assert_eq!(out.errors.len(), 1);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind_name()).collect::<Vec<_>>(),
        ["Id", "Error", "Id", "Eof"]
    );
    // Error position is exact for single characters.
    assert_eq!((out.errors[0].line, out.errors[0].col), (1, 3));
}

#[test]
fn error_line_tracking() {
    let (out, _it) = lexed(FileId(0), "ab\ncd ` ");
    assert_eq!(out.errors.len(), 1);
    assert_eq!((out.errors[0].line, out.errors[0].col), (2, 4));
}

#[test]
fn doc_comments_are_tokens() {
    let (out, mut it) = lexed(FileId(0), "/// hello\n/// world\nx");
    assert!(out.is_ok());
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::DocLine(sym(&mut it, " hello\n world"))
    );
    let (out, mut it) = lexed(FileId(0), "/*! hi */ x");
    assert!(out.is_ok());
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::DocBlock(sym(&mut it, " hi "))
    );
    // `code_tokens` skips docs.
    assert_eq!(out.code_tokens().len(), 2); // Id(x) + Eof
}

#[test]
fn payloads_are_interned_symbols() {
    // Lane 71: the same spelling lexes to the same symbol (one table entry
    // per distinct string, not one `String` per occurrence).
    let (out, mut it) = lexed(FileId(0), "foo foo 42 foo");
    assert!(out.is_ok());
    let ids: Vec<Symbol> = out
        .tokens
        .iter()
        .filter_map(|t| match t.kind {
            TokenKind::Id(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(ids[1], ids[2]);
    assert_eq!(it.resolve(ids[0]), Some("foo"));
    let nums: Vec<Symbol> = out
        .tokens
        .iter()
        .filter_map(|t| match t.kind {
            TokenKind::Num(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(nums.len(), 1);
    assert_eq!(it.resolve(nums[0]), Some("42"));
    // Exactly two distinct payload strings share the table.
    assert_eq!(it.len(), 2);
    // Re-interning the expectation hits the same symbols.
    assert_eq!(TokenKind::Id(sym(&mut it, "foo")), out.tokens[0].kind);
}

#[test]
fn empty_input_is_just_eof() {
    let (out, _it) = lexed(FileId(3), "");
    assert!(out.is_ok());
    assert_eq!(out.tokens.len(), 1);
    assert_eq!(out.tokens[0].span, Span::new(FileId(3), 0, 0));
}

#[test]
fn lexer_types_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Token>();
    assert_send_sync::<TokenKind>();
    assert_send_sync::<LexError>();
    assert_send_sync::<LexResult>();
    assert_send_sync::<Span>();
    assert_send_sync::<FileId>();
    assert_send_sync::<Symbol>();
}
