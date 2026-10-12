//! Lane-37: scattered-clause body elaboration against a source-position
//! view (mirrors `declare_scattered` linkage; bodies get plans instead of
//! the blanket `Unsupported`).

use sail_ast::*;
use sail_ty::scattered_elab::{BodyPlan, Namespace, ScatteredView, elaborate, namespace_of};
use std::collections::BTreeMap;

#[derive(Default)]
struct Fake {
    open: BTreeMap<(Namespace, String), bool>,
    counts: BTreeMap<(Namespace, String), usize>,
    closed: BTreeMap<String, Namespace>,
}

impl ScatteredView for Fake {
    fn is_open(&self, namespace: Namespace, name: &str) -> bool {
        self.open
            .get(&(namespace, name.to_owned()))
            .copied()
            .unwrap_or(false)
    }

    fn clause_count(&self, namespace: Namespace, name: &str) -> usize {
        self.counts
            .get(&(namespace, name.to_owned()))
            .copied()
            .unwrap_or(0)
    }

    fn closed_at(&self, name: &str, _position: usize) -> Option<Namespace> {
        self.closed.get(name).copied()
    }
}

fn interner() -> SimpleInterner {
    SimpleInterner::new()
}

fn name(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn funcl(it: &mut SimpleInterner, fname: &str) -> Funcl {
    Funcl {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(it, fname),
        arm: PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: Node::dummy(PatKind::Wild),
                body: Node::dummy(ExpKind::Undef),
            },
        },
    }
}

#[test]
fn scattered_block_lifecycle_elaborates_without_unsupported() {
    let mut it = interner();
    let open = Node::dummy(DefKind::Scattered(ScatteredDef::Function {
        name: name(&mut it, "f"),
        annot: TAnnotOpt::None,
    }));
    let clause = Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl(
        &mut it, "f",
    ))));
    let end = Node::dummy(DefKind::Scattered(ScatteredDef::End(name(
        &mut it, "f",
    ))));
    let it = it;

    // Opening: no body work, linkage accepted.
    assert_eq!(
        elaborate(&open, &it, &Fake::default(), 0).unwrap(),
        BodyPlan::AcceptHeader {
            namespace: Namespace::Function,
            name: "f".to_owned(),
        }
    );

    // Two clauses elaborate to check-plans with 0-based indices (body-time
    // counts include each clause's own declare entry).
    let one = Fake {
        open: [((Namespace::Function, "f".to_owned()), true)]
            .into_iter()
            .collect(),
        counts: [((Namespace::Function, "f".to_owned()), 1)]
            .into_iter()
            .collect(),
        closed: BTreeMap::new(),
    };
    assert_eq!(
        elaborate(&clause, &it, &one, 1).unwrap(),
        BodyPlan::CheckFuncl {
            name: "f".to_owned(),
            index: 0,
        }
    );
    let two = Fake {
        open: [((Namespace::Function, "f".to_owned()), true)]
            .into_iter()
            .collect(),
        counts: [((Namespace::Function, "f".to_owned()), 2)]
            .into_iter()
            .collect(),
        closed: BTreeMap::new(),
    };
    assert_eq!(
        elaborate(&clause, &it, &two, 2).unwrap(),
        BodyPlan::CheckFuncl {
            name: "f".to_owned(),
            index: 1,
        }
    );

    // End resolves the recorded close; no `Unsupported` anywhere.
    let done = Fake {
        open: [((Namespace::Function, "f".to_owned()), false)]
            .into_iter()
            .collect(),
        counts: [((Namespace::Function, "f".to_owned()), 2)]
            .into_iter()
            .collect(),
        closed: [(("f".to_owned()), Namespace::Function)]
            .into_iter()
            .collect(),
    };
    assert_eq!(
        elaborate(&end, &it, &done, 3).unwrap(),
        BodyPlan::AcceptEnd {
            namespace: Namespace::Function,
            name: "f".to_owned(),
        }
    );
}

#[test]
fn orphan_scattered_clause_still_fails_with_quant_error() {
    let mut it = interner();
    let clause = Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl(
        &mut it, "ghost",
    ))));
    let it = it;
    let err = elaborate(&clause, &it, &Fake::default(), 0).unwrap_err();
    assert!(
        matches!(
            err.kind,
            sail_ty::error::TypeErrorKind::QuantError(_)
        ),
        "unexpected: {err:?}"
    );
    assert!(namespace_of(&ScatteredDef::End(Name::dummy(Symbol::DUMMY))).is_none());
}
