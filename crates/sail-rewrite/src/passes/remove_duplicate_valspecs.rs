//! Duplicate signature removal, matching `remove_duplicate_valspecs` in
//! `sail/src/lib/rewrites.ml`.
//!
//! OCaml keeps the *first* `val` spec per name but attaches the *last*
//! extern block (so backend externs win while the earliest source position
//! is retained). This pass does exactly that on the canonical AST, keyed by
//! interned [`Symbol`](sail_ast::Symbol) equality.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{Ast, DefKind, Extern};

use crate::pass::TypedPass;

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoveDuplicateValspecs;

impl TypedPass for RemoveDuplicateValspecs {
    fn name(&self) -> &'static str { "remove_duplicate_valspecs" }

    fn run(&self, ast: &mut Ast, _interner: &mut dyn sail_ast::Interner) {
        // Last extern block per name, as in OCaml `last_externs`.
        let mut last_externs: BTreeMap<u32, Option<Extern>> = BTreeMap::new();
        for def in &ast.defs {
            if let DefKind::Val(vs) = &def.kind {
                last_externs.insert(vs.name.symbol.0, vs.extern_opt.clone());
            }
        }
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut kept = Vec::with_capacity(ast.defs.len());
        for mut def in std::mem::take(&mut ast.defs) {
            if let DefKind::Val(ref mut vs) = def.kind {
                if !seen.insert(vs.name.symbol.0) {
                    continue;
                }
                if let Some(ext) = last_externs.remove(&vs.name.symbol.0).flatten() {
                    vs.extern_opt = Some(ext);
                }
            }
            kept.push(def);
        }
        ast.defs = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{Def, Interner, Node, QuantItem, SimpleInterner, Span, TypKind, TypSchm};

    use crate::pass::TypedPass;

    fn valspec(
        it: &mut SimpleInterner,
        name: &str,
        backend: Option<(&str, &str)>,
    ) -> Def {
        let sym = it.intern(name);
        Def::dummy(DefKind::Val(sail_ast::ValSpec {
            id: sail_ast::NodeId::DUMMY,
            span: Span::DUMMY,
            name: sail_ast::Name::dummy(sym),
            typschm: TypSchm {
                quant: Vec::<QuantItem>::new(),
                typ: Node::dummy(TypKind::Unknown),
            },
            extern_opt: backend.map(|(k, v)| Extern {
                pure: true,
                bindings: vec![(k.to_owned(), v.to_owned())],
            }),
        }))
    }

    #[test]
    fn keeps_first_valspec_with_last_extern() {
        let mut it = SimpleInterner::new();
        let mut ast = Ast::new(vec![
            valspec(&mut it, "f", Some(("lean", "one"))),
            valspec(&mut it, "g", None),
            valspec(&mut it, "f", Some(("lean", "two"))),
        ]);
        RemoveDuplicateValspecs.run(&mut ast, &mut it);
        assert_eq!(ast.defs.len(), 2);
        let DefKind::Val(first) = &ast.defs[0].kind else {
            panic!("def shape")
        };
        assert_eq!(it.resolve(first.name.symbol), Some("f"));
        let ext = first.extern_opt.as_ref().expect("extern kept");
        assert_eq!(ext.bindings, vec![("lean".to_owned(), "two".to_owned())]);
        // Idempotent.
        let snapshot = ast.clone();
        RemoveDuplicateValspecs.run(&mut ast, &mut it);
        assert_eq!(ast, snapshot);
    }
}
