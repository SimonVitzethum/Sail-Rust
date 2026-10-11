use sail_grammar::lexer::{tokenize, Tok};

#[test]
fn wildcard_is_distinct_from_underscore_prefixed_identifiers() {
    let tokens: Vec<_> = tokenize("_ _R __deref").unwrap()
        .into_iter().map(|(_, token, _)| token).collect();
    assert_eq!(tokens, vec![Tok::Under, Tok::Id("_R".into()), Tok::Id("__deref".into())]);
    assert!(matches!(sail_grammar::parse_pat("_").unwrap(), sail_grammar::ast::Pat::Wild));
    assert!(matches!(sail_grammar::parse_typ("_").unwrap(), sail_grammar::ast::Typ::Wild));
}

#[test]
fn longest_operator_match_preserves_reserved_tokens_and_suffixes() {
    let tokens: Vec<_> = tokenize("= == >= >=_s != / /_u ^^ @ ^ :: : .. . * ** | || => -> <-> - --")
        .unwrap().into_iter().map(|(_, token, _)| token).collect();
    let op = |s: &str| Tok::OpId(s.into());
    assert_eq!(tokens, vec![Tok::Eq, op("=="), op(">="), op(">=_s"), op("!="),
        op("/"), op("/_u"), op("^^"), Tok::At, Tok::Caret, Tok::ColonColon,
        Tok::Colon, Tok::DotDot, Tok::Dot, Tok::Star, op("**"), Tok::Bar,
        op("||"), Tok::EqGt, Tok::MinusGt, Tok::Bidir, Tok::Minus, op("--")]);
}

#[test]
fn comments_and_collection_closers_keep_their_lexical_roles() {
    let tokens: Vec<_> = tokenize("/ /* nested /* comment */ done */ // line\n |] |}")
        .unwrap().into_iter().map(|(_, token, _)| token).collect();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0], Tok::OpId("/".into()));
    // Check by parsing the corresponding collection forms as well: a token
    // spelling that is merely another operator cannot close a collection.
    assert!(sail_grammar::parse_exp("[|1, 2|]").is_ok());
    assert!(sail_grammar::parse_typ("{|1, 2|}").is_ok());
}
