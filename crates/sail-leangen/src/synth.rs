#![forbid(unsafe_code)]
//! Definitions the OCaml pipeline synthesises: `initialize_registers`
//! (`Initial_check.generate_initialize_registers`) and `sail_model_init`
//! (`State.add_register_init_function`).

use std::sync::Arc;

use sail_ast::{
    Annotation, Def, DefKind, Exp, ExpKind, Funcl, Interner, LExp, LExpKind, Lit, LitKind, Name, Node, NodeId, PExp,
    PExpKind, Pat, PatKind, RecOpt, SimpleInterner, Span, TAnnotOpt, Typ, TypKind, TypSchm, ValSpec,
};

use crate::remap::Remap;
use crate::rw::typs::with_typ;

/// A register declaration with its (elaborated) initialiser.
pub struct RegDecl {
    pub name: String,
    pub typ: Typ,
    pub it: Arc<SimpleInterner>,
    pub init: Option<Exp>,
}

fn name(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn unit_typ(it: &mut SimpleInterner) -> Typ {
    Node::dummy(TypKind::Id(name(it, "unit")))
}

fn typed<K>(kind: K, typ: &Typ) -> Node<K> {
    with_typ(Node::dummy(kind), Some(typ.clone()))
}

fn unit_exp(it: &mut SimpleInterner) -> Exp {
    let t = unit_typ(it);
    typed(ExpKind::Lit(Lit::dummy(LitKind::Unit)), &t)
}

fn val_def(it: &mut SimpleInterner, fname: &str) -> Def {
    let u = unit_typ(it);
    let typ = Node::dummy(TypKind::Fn { args: vec![u.clone()], ret: Box::new(u) });
    Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(it, fname),
        typschm: TypSchm { quant: Vec::new(), typ },
        extern_opt: None,
    }))
}

fn fun_def(it: &mut SimpleInterner, fname: &str, pat: Pat, body: Exp) -> Def {
    let funcl = Funcl {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(it, fname),
        arm: PExp { id: NodeId::DUMMY, span: Span::DUMMY, kind: PExpKind::PatExp { pat, body } },
    };
    Node::dummy(DefKind::Fundef { rec: RecOpt::NonRec, annot: TAnnotOpt::None, funcls: vec![funcl] })
}

fn is_option(typ: &Typ, it: &SimpleInterner) -> bool {
    matches!(&typ.kind, TypKind::App { name, .. } if Interner::resolve(it, name.symbol) == Some("option"))
}

/// The four definitions (val + function for each), each with its interner.
#[must_use]
pub fn init_defs(regs: &[RegDecl]) -> Vec<(Def, SimpleInterner)> {
    let mut out = Vec::new();

    // initialize_registers
    {
        let mut it = SimpleInterner::new();
        let unit = unit_typ(&mut it);
        let mut assigns = Vec::new();
        for r in regs.iter().filter(|r| r.init.is_none() && !is_option(&r.typ, &r.it)) {
            let typ = Remap { from: &r.it, to: &mut it }.typ(&r.typ);
            let rid = name(&mut it, &r.name);
            let lexp: LExp = typed(LExpKind::Id(rid), &typ);
            let undef = typed(ExpKind::Undef, &typ);
            assigns.push(typed(ExpKind::Assign { lexp: Box::new(lexp), value: Box::new(undef) }, &unit));
        }
        let body = if assigns.is_empty() { unit_exp(&mut it) } else { typed(ExpKind::Block(assigns), &unit) };
        let pat = typed(PatKind::Lit(Lit::dummy(LitKind::Unit)), &unit);
        let val = val_def(&mut it, "initialize_registers");
        let fun = fun_def(&mut it, "initialize_registers", pat, body);
        out.push((val, it.clone()));
        out.push((fun, it));
    }

    // sail_model_init
    {
        let mut it = SimpleInterner::new();
        let unit = unit_typ(&mut it);
        let mut stmts = Vec::new();
        for r in regs {
            if let Some(init) = &r.init {
                let typ = Remap { from: &r.it, to: &mut it }.typ(&r.typ);
                let value = Remap { from: &r.it, to: &mut it }.exp(init);
                let rid = name(&mut it, &r.name);
                let lexp: LExp = typed(LExpKind::Id(rid), &typ);
                stmts.push(typed(ExpKind::Assign { lexp: Box::new(lexp), value: Box::new(value) }, &unit));
            }
        }
        let func = name(&mut it, "initialize_registers");
        let arg = unit_exp(&mut it);
        stmts.push(typed(ExpKind::App { func, args: vec![arg] }, &unit));
        let body = typed(ExpKind::Block(stmts), &unit);
        let pat = typed(PatKind::Wild, &unit);
        let val = val_def(&mut it, "sail_model_init");
        let fun = fun_def(&mut it, "sail_model_init", pat, body);
        out.push((val, it.clone()));
        out.push((fun, it));
    }
    let _: Option<Annotation> = None;
    out
}
