#![forbid(unsafe_code)]
//! JSON snapshot helpers for [`crate::ast::Ast`].

use crate::ast::Ast;

/// Serialize an AST to pretty JSON (stable snapshots).
pub fn to_string(ast: &Ast) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(ast)
}

/// Compact JSON for hashing / wire transfer.
pub fn to_string_compact(ast: &Ast) -> Result<String, serde_json::Error> {
    serde_json::to_string(ast)
}

/// Deserialize an AST from JSON.
pub fn from_str(text: &str) -> Result<Ast, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interner as _, NodeId, SimpleInterner, Span};

    #[test]
    fn empty_ast_roundtrip() {
        let ast = Ast::new(Vec::new());
        let text = to_string(&ast).unwrap();
        let back = from_str(&text).unwrap();
        assert_eq!(ast, back);
    }

    #[test]
    fn invalid_json_is_error() {
        assert!(from_str("{ not json").is_err());
    }

    #[test]
    fn interner_survives_alongside_ast_json() {
        let mut interner = SimpleInterner::new();
        let sym = interner.intern("my_fun");
        assert_eq!(interner.resolve(sym), Some("my_fun"));
        let ast = Ast::new(Vec::new());
        let _ = NodeId::DUMMY;
        let _ = Span::DUMMY;
        let text = to_string_compact(&ast).unwrap();
        assert_eq!(from_str(&text).unwrap(), ast);
    }
}
