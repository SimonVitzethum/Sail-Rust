
#![forbid(unsafe_code)]
//! Behaviour tests for the lexer, pinned against `lexer.mll`.

use sail_lex::{FileId, Fixity, LexError, LexResult, Span, Token, TokenKind, lex};

fn kinds(src: &str) -> Vec<TokenKind> {
    lex(FileId(0), src).tokens.into_iter().map(|t| t.kind).collect()
}

fn id(s: &str) -> TokenKind {
    TokenKind::Id(s.to_string())
}

fn op(s: &str) -> TokenKind {
    TokenKind::Op(s.to_string())
}

#[test]
fn basic_function_shape_and_spans() {
    let out = lex(FileId(7), "function foo(x) = { x }");
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
    assert_eq!(
        kinds("match matchy Type Typey true truthy undefined undef"),
        [
            TokenKind::Match,
            id("matchy"),
            TokenKind::KindType,
            id("Typey"),
            TokenKind::True,
            id("truthy"),
            TokenKind::Undefined,
            id("undef"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn tilde_is_plain_id_like_ocaml() {
    assert_eq!(kinds("~"), [id("~"), TokenKind::Eof]);
}

#[test]
fn underscore_and_unit() {
    assert_eq!(
        kinds("_ _x () ( )"),
        [
            TokenKind::Under,
            id("_x"),
            TokenKind::Unit,
            TokenKind::Lparen,
            TokenKind::Rparen,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn tyvar_and_lone_quote_error() {
    assert_eq!(
        kinds("'a 'foo"),
        [
            TokenKind::TyVar("'a".to_string()),
            TokenKind::TyVar("'foo".to_string()),
            TokenKind::Eof,
        ]
    );
    let out = lex(FileId(0), "a ' b");
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains('\''));
    assert_eq!(
        kinds("a ' b"),
        [id("a"), TokenKind::Error("Unexpected character: `'`".to_string()), id("b"), TokenKind::Eof]
    );
}

#[test]
fn numbers_reals_ranges() {
    assert_eq!(
        kinds("42 1.5 1..2 1. 0"),
        [
            TokenKind::Num("42".to_string()),
            TokenKind::Real("1.5".to_string()),
            TokenKind::Num("1".to_string()),
            TokenKind::DotDot,
            TokenKind::Num("2".to_string()),
            TokenKind::Num("1".to_string()),
            TokenKind::Dot,
            TokenKind::Num("0".to_string()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn bit_literals_hex_bin() {
    let out = lex(FileId(0), "0xFF 0b101 0xFFFF_0000_1234_0000 0b1_0");
    assert!(out.is_ok());
    assert_eq!(
        kinds("0xFF 0b101 0xFFFF_0000_1234_0000 0b1_0"),
        [
            TokenKind::Hex("FF".to_string()),
            TokenKind::Bin("101".to_string()),
            TokenKind::Hex("FFFF_0000_1234_0000".to_string()),
            TokenKind::Bin("1_0".to_string()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn bare_0x_falls_back_to_num_plus_id() {
    // Mirrors ocamllex: `0x` needs hex digits, otherwise `Num("0") Id(...)`.
    assert_eq!(
        kinds("0x + 1"),
        [
            TokenKind::Num("0".to_string()),
            id("x"),
            op("+"),
            TokenKind::Num("1".to_string()),
            TokenKind::Eof,
        ]
    );
    assert_eq!(
        kinds("0b2"),
        [
            TokenKind::Num("0".to_string()),
            id("b2"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn two_caret_forms() {
    assert_eq!(
        kinds("2^3"),
        [TokenKind::TwoCaret, TokenKind::Num("3".to_string()), TokenKind::Eof]
    );
    assert_eq!(
        kinds("2 ^ 3"),
        [TokenKind::TwoCaret, TokenKind::Num("3".to_string()), TokenKind::Eof]
    );
    // Longer digit runs are `Num` + `Caret`.
    assert_eq!(
        kinds("12^3"),
        [
            TokenKind::Num("12".to_string()),
            TokenKind::Caret,
            TokenKind::Num("3".to_string()),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn comments_are_skipped_and_nest() {
    let out = lex(FileId(0), "a /* x /* y */ z */ b // tail\nc");
    assert!(out.is_ok());
    assert_eq!(out.tokens.iter().filter(|t| !t.is_eof()).count(), 3);
    assert_eq!(kinds("a /* x /* y */ z */ b // tail\nc"), [id("a"), id("b"), id("c"), TokenKind::Eof]);
}

#[test]
fn paren_star_comment_needs_whitespace() {
    // Lane-spec `(* *)` comments …
    assert_eq!(kinds("a (* hello *) b"), [id("a"), id("b"), TokenKind::Eof]);
    assert_eq!(
        kinds("a (*\nhello\n*) b"),
        [id("a"), id("b"), TokenKind::Eof]
    );
    // … while real code like `(*TVAL)` still lexes as `(` `*`.
    assert_eq!(
        kinds("(*TVAL) = x"),
        [
            TokenKind::Lparen,
            TokenKind::Star,
            id("TVAL"),
            TokenKind::Rparen,
            TokenKind::Eq,
            id("x"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn unbalanced_comment_is_recoverable() {
    let out = lex(FileId(0), "a /* oops");
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains("Unbalanced"));
    assert!(out.tokens.last().is_some_and(|t| t.is_eof()));
}

#[test]
fn stray_comment_closer_is_an_error() {
    let out = lex(FileId(0), "a */ b");
    assert_eq!(out.errors.len(), 1);
    assert_eq!(
        kinds("a */ b"),
        [
            id("a"),
            TokenKind::Error("Unbalanced comment: stray `*/`".to_string()),
            id("b"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn strings_with_escapes() {
    let out = lex(FileId(0), r#""a\nb\"q\"\\A\x41\065""#);
    assert!(out.is_ok(), "errors: {:?}", out.errors);
    assert_eq!(
        kinds(r#""a\nb\"q\"\\A\x41\065""#),
        [TokenKind::Str("a\nb\"q\"\\AAA".to_string()), TokenKind::Eof]
    );
}

#[test]
fn string_continuation_and_raw_newline() {
    let out = lex(FileId(0), "\"a\\\n   b\"");
    assert!(out.is_ok());
    assert_eq!(kinds("\"a\\\n   b\""), [TokenKind::Str("ab".to_string()), TokenKind::Eof]);

    let out = lex(FileId(0), "\"a\nb\"");
    assert!(out.is_ok());
    assert_eq!(kinds("\"a\nb\""), [TokenKind::Str("a\nb".to_string()), TokenKind::Eof]);
}

#[test]
fn unterminated_string_recovers() {
    let out = lex(FileId(0), "\"abc");
    assert_eq!(out.errors.len(), 1);
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::Str("abc".to_string()),
    );
    assert!(out.tokens.last().is_some_and(|t| t.is_eof()));
}

#[test]
fn illegal_escape_recovers_inline() {
    let out = lex(FileId(0), "\"a\\qb\"");
    assert_eq!(out.errors.len(), 1);
    // Policy: the offending span leaves an `Error` token behind; the string
    // itself continues and is still emitted.
    assert!(matches!(out.tokens[0].kind, TokenKind::Error(_)));
    assert_eq!(out.tokens[1].kind, TokenKind::Str("aqb".to_string()));
}

#[test]
fn multiline_string_strips_prefix() {
    let src = "\"\"\"\n  hello\n\n  world\n  \"\"\"";
    let out = lex(FileId(0), src);
    assert!(out.is_ok(), "errors: {:?}", out.errors);
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::MultiLineStr("hello\n\nworld".to_string())
    );
}

#[test]
fn multiline_string_bad_prefix_is_an_error() {
    let src = "\"\"\"\n  hello\nnoprefix\n  \"\"\"";
    let out = lex(FileId(0), src);
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains("prefix"));
}

#[test]
fn operators_and_arrows() {
    assert_eq!(
        kinds("x == y => z -> w <-> v ==> u"),
        [
            id("x"),
            op("=="),
            id("y"),
            TokenKind::EqGt,
            id("z"),
            TokenKind::MinusGt,
            id("w"),
            TokenKind::Bidir,
            id("v"),
            op("==>"),
            id("u"),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn long_arrow_becomes_operator() {
    // Longest match wins, as in ocamllex: `a-->b` is `Id Op("-->") Id`.
    assert_eq!(
        kinds("a-->b"),
        [id("a"), op("-->"), id("b"), TokenKind::Eof]
    );
}

#[test]
fn slash_before_comment_ends_operator() {
    assert_eq!(
        kinds("+/*c*/b //x\n"),
        [op("+"), id("b"), TokenKind::Eof]
    );
}

#[test]
fn colons_dots_pipes() {
    assert_eq!(
        kinds(":: ::: := : .. ... . | || [| |] {| |}"),
        [
            TokenKind::ColonColon,
            op(":::"),
            op(":="),
            TokenKind::Colon,
            TokenKind::DotDot,
            op("..."),
            TokenKind::Dot,
            TokenKind::Bar,
            op("||"),
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
    assert_eq!(
        kinds("infixl 8 ++"),
        [
            TokenKind::Fixity {
                fixity: Fixity::InfixL,
                level: 8,
                op: "++".to_string(),
            },
            TokenKind::Eof,
        ]
    );
    assert_eq!(
        kinds("infix 4 <->"),
        [
            TokenKind::Fixity {
                fixity: Fixity::Infix,
                level: 4,
                op: "<->".to_string(),
            },
            TokenKind::Eof,
        ]
    );
    // Without `N op` these are plain identifiers.
    assert_eq!(
        kinds("infix infixl 8 infixr"),
        [id("infix"), id("infixl"), TokenKind::Num("8".to_string()), id("infixr"), TokenKind::Eof]
    );
}

#[test]
fn directives_pragmas_attributes() {
    let out = lex(FileId(0), "$include <prelude.sail>\n");
    assert!(out.is_ok());
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::Pragma {
            name: "include".to_string(),
            value: " <prelude.sail>".to_string(),
        }
    );
    assert_eq!(
        kinds("$foo {"),
        [TokenKind::StructuredPragma("foo".to_string()), TokenKind::Eof]
    );
    assert_eq!(
        kinds("$[mycfg]"),
        [
            TokenKind::Attribute("mycfg".to_string()),
            TokenKind::Rsquare,
            TokenKind::Eof,
        ]
    );
    let out = lex(FileId(0), "$\n");
    assert_eq!(out.errors.len(), 1);
}

#[test]
fn reserved_words_are_errors_not_idents() {
    for word in ["import", "module", "where"] {
        let out = lex(FileId(0), word);
        assert_eq!(out.errors.len(), 1, "{word}");
        assert!(out.errors[0].message.contains("reserved"), "{word}");
        assert!(matches!(out.tokens[0].kind, TokenKind::Error(_)), "{word}");
    }
}

#[test]
fn unexpected_characters_recover() {
    let out = lex(FileId(0), "a ` b");
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
    let out = lex(FileId(0), "ab\ncd ` ");
    assert_eq!(out.errors.len(), 1);
    assert_eq!((out.errors[0].line, out.errors[0].col), (2, 4));
}

#[test]
fn doc_comments_are_tokens() {
    let out = lex(FileId(0), "/// hello\n/// world\nx");
    assert!(out.is_ok());
    assert_eq!(
        out.tokens[0].kind,
        TokenKind::DocLine(" hello\n world".to_string())
    );
    let out = lex(FileId(0), "/*! hi */ x");
    assert!(out.is_ok());
    assert_eq!(out.tokens[0].kind, TokenKind::DocBlock(" hi ".to_string()));
    // `code_tokens` skips docs.
    assert_eq!(out.code_tokens().len(), 2); // Id(x) + Eof
}

#[test]
fn empty_input_is_just_eof() {
    let out = lex(FileId(3), "");
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
}
