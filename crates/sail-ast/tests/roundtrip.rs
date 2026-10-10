#![forbid(unsafe_code)]
//! End-to-end snapshot test: build a small typed AST, JSON-roundtrip it,
//! and share it across threads (`Send + Sync` proof for parallel lanes).

use sail_ast::{
    Ast, Constraint, ConstraintKind, Def, DefKind, Exp, ExpKind, Funcl, IdGen, Interner as _,
    LitKind, Mapcl, MapclKind, Name, NExp, NExpKind, Node, NodeId, Order, PExp, PExpKind, Pat,
    PatKind, QuantItem, RecOpt, SimpleInterner, Span, TAnnotOpt, Typ, TypKind, TypSchm, ValSpec,
};

struct Ctx {
    ids: IdGen,
    interner: SimpleInterner,
}

impl Ctx {
    fn new() -> Self {
        Ctx {
            ids: IdGen::new(),
            interner: SimpleInterner::new(),
        }
    }

    fn name(&mut self, text: &str) -> Name {
        let symbol = self.interner.intern(text);
        Name::new(self.ids.fresh(), Span::dummy(), symbol, false)
    }

    fn exp_id(&mut self, text: &str) -> Exp {
        let name = self.name(text);
        Node::new(self.ids.fresh(), Span::dummy(), ExpKind::Id(name))
    }

    fn pat_id(&mut self, text: &str) -> Pat {
        let name = self.name(text);
        Node::new(self.ids.fresh(), Span::dummy(), PatKind::Id(name))
    }
}

fn sample_ast() -> Ast {
    let mut ctx = Ctx::new();

    // val my_id : forall 'a. 'a -> 'a
    let alpha = ctx.interner.intern("'a");
    let var_a: Typ = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        TypKind::Var(alpha),
    );
    let ret_a: Typ = Node::new(ctx.ids.fresh(), Span::dummy(), TypKind::Var(alpha));
    let fun_ty: Typ = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        TypKind::Fn {
            args: vec![var_a],
            ret: Box::new(ret_a),
        },
    );
    let val_name = ctx.name("my_id");
    let val = ValSpec {
        id: ctx.ids.fresh(),
        span: Span::dummy(),
        name: val_name,
        typschm: TypSchm {
            quant: vec![QuantItem::Id(sail_ast::KOpt {
                kind: sail_ast::Kind::Type,
                kid: alpha,
            })],
            typ: fun_ty,
        },
        extern_opt: None,
    };
    let val_def: Def = Node::new(ctx.ids.fresh(), Span::dummy(), DefKind::Val(val));

    // fn my_id(x) = match x { _ => x }
    let param = ctx.pat_id("x");
    let wild: Pat = Node::new(ctx.ids.fresh(), Span::dummy(), PatKind::Wild);
    let arm_body = ctx.exp_id("x");
    let arm = PExp {
        id: ctx.ids.fresh(),
        span: Span::dummy(),
        kind: PExpKind::PatExp {
            pat: wild,
            body: arm_body,
        },
    };
    let scrutinee = ctx.exp_id("x");
    let match_exp: Exp = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        ExpKind::Match {
            scrutinee: Box::new(scrutinee),
            arms: vec![arm],
        },
    );
    let fun_name = ctx.name("my_id");
    let fun_pat: Pat = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        PatKind::Tuple(vec![param]),
    );
    let fun_arm = PExp {
        id: ctx.ids.fresh(),
        span: Span::dummy(),
        kind: PExpKind::PatExp {
            pat: fun_pat,
            body: match_exp,
        },
    };
    let funcl = Funcl {
        id: ctx.ids.fresh(),
        span: Span::dummy(),
        name: fun_name,
        arm: fun_arm,
    };
    let fun_def: Def = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl],
        },
    );

    // let _ : bool = true  (exercises Lit + Let + Constraint::True)
    let lit_true: sail_ast::Lit = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        LitKind::True,
    );
    let lit_exp: Exp = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        ExpKind::Lit(lit_true),
    );
    let wild_pat: Pat = Node::new(ctx.ids.fresh(), Span::dummy(), PatKind::Wild);
    let tail: Exp = Node::new(ctx.ids.fresh(), Span::dummy(), ExpKind::Undef);
    let let_exp: Exp = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        ExpKind::Let {
            pat: wild_pat,
            bind: Box::new(lit_exp),
            body: Box::new(tail),
        },
    );
    let top_pat: Pat = Node::new(ctx.ids.fresh(), Span::dummy(), PatKind::Wild);
    let let_def: Def = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        DefKind::Let {
            pat: top_pat,
            exp: let_exp,
        },
    );

    // constraint + nexp + mapdef (exercises NExp/Constraint/Mapcl/Order/NodeId::DUMMY)
    let n8: NExp = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        NExpKind::Constant("8".to_owned()),
    );
    let n0: NExp = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        NExpKind::Constant("0".to_owned()),
    );
    let ge: Constraint = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        ConstraintKind::Ge(n8, n0),
    );
    let c_def: Def = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        DefKind::Constraint(ge),
    );
    let map_name = ctx.name("my_map");
    let fwd_pat: Pat = Node::new(NodeId::DUMMY, Span::dummy(), PatKind::Wild);
    let fwd_body: Exp = Node::new(NodeId::DUMMY, Span::dummy(), ExpKind::Undef);
    let fwd = PExp {
        id: NodeId::DUMMY,
        span: Span::dummy(),
        kind: PExpKind::PatExp {
            pat: fwd_pat,
            body: fwd_body,
        },
    };
    let mapcl = Mapcl {
        id: ctx.ids.fresh(),
        span: Span::dummy(),
        kind: MapclKind::Forwards(fwd),
    };
    let map_def: Def = Node::new(
        ctx.ids.fresh(),
        Span::dummy(),
        DefKind::Mapdef {
            name: map_name,
            annot: TAnnotOpt::None,
            clauses: vec![mapcl],
        },
    );
    let _ = Order::Inc; // keep Order linked into the snapshot surface

    Ast::new(vec![val_def, fun_def, let_def, c_def, map_def])
}

#[test]
fn json_roundtrip_preserves_ast() {
    let ast = sample_ast();
    let text = ast.to_json().expect("serialize");
    let back = Ast::from_json(&text).expect("deserialize");
    assert_eq!(ast, back);
}

#[test]
fn compact_json_roundtrips_too() {
    let ast = sample_ast();
    let text = sail_ast::json::to_string_compact(&ast).expect("serialize");
    let back = sail_ast::json::from_str(&text).expect("deserialize");
    assert_eq!(ast, back);
}

#[test]
fn ast_is_shared_across_threads() {
    let ast = sample_ast();
    let text = ast.to_json().expect("serialize");
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    // Read-only sharing: requires `Ast: Send + Sync`.
                    let local = Ast::from_json(&text).expect("deserialize");
                    assert_eq!(local.defs.len(), 5);
                    local.defs.len()
                })
            })
            .collect();
        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(total, 8 * 5);
    });
    // Original untouched.
    assert_eq!(ast.defs.len(), 5);
}
