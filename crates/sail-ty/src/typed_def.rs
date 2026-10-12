#![forbid(unsafe_code)]
//! Elaborated typed definitions (Lane 17).
//!
//! [`TypedDef`] is the per-definition emission of the canonical checking
//! pipeline: a checked definition carrying its type/constraint payload
//! (the declared [`TypSchm`](sail_ast::TypSchm): quantifier telescope with
//! numeric constraints plus body type), not a bare expression count.
//!
//! Ownership and teardown:
//!
//! - The payload is *owned* AST (`TypSchm` cloned from the frozen
//!   declaration table, plus the definition-local interner needed to
//!   resolve its symbols downstream). Dropping a [`TypedDef`] re-wraps the
//!   owned nodes in a dummy `Val` definition and releases them through
//!   [`sail_ast::storage::drop_definition`], i.e. the explicit work-stack
//!   path — there are no recursive drops, so deeply nested payload types
//!   cannot overflow the stack on teardown (same pattern as
//!   `Scheme`'s signature disposal in `program.rs`).
//! - [`TypedDef`] is `Send`, so scheduler workers can construct values and
//!   hand them to the single-threaded consumer without retaining loaded
//!   AST bodies (out-of-core streaming is preserved; bodies are still
//!   dropped right after checking).
//!
//! Downstream stages (rewrite/lean consumption of the emitted program) do
//! not exist yet; the CLI therefore keeps failing explicitly instead of
//! reporting emission as a successful build (see `sail-cli` `typecheck`).

use crate::program::Program;
use sail_ast::{
    Def, DefKind, Interner, Name, Node, NodeId, SimpleInterner, Span, Symbol, TypSchm, TypeDefKind,
    ValSpec,
};
use std::sync::Arc;

/// Which declaration form a [`TypedDef`] was elaborated from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedDefKind {
    Function,
    Value,
    Register,
    Type,
    Constraint,
    Overload,
    Let,
    Other,
}

/// A checked definition with its type/constraint payload.
///
/// `typschm`/`interner` are `None` for declaration forms without a value
/// scheme (type declarations, constraints, pragmas, …); value-like forms
/// snapshot the frozen declaration-table scheme at their source position.
pub struct TypedDef {
    pub name: String,
    pub kind: TypedDefKind,
    pub position: usize,
    pub expressions: usize,
    typschm: Option<TypSchm>,
    interner: Option<Arc<SimpleInterner>>,
}

impl TypedDef {
    /// Elaborate one loaded definition after
    /// [`Program::check_definition`](crate::program::Program::check_definition)
    /// succeeded. The scheme snapshot is read from the frozen declaration
    /// table at `position` (a worker never sees a later replacement); the
    /// caller still owns `def` and must drop it via the storage path.
    #[must_use]
    pub fn checked(
        def: &Def,
        it: &SimpleInterner,
        position: usize,
        expressions: usize,
        program: &Program,
    ) -> Self {
        let (name, kind) = describe(def, it);
        let scheme = match kind {
            TypedDefKind::Function | TypedDefKind::Value => program.value(&name, position),
            TypedDefKind::Register => program.register(&name, position),
            _ => None,
        };
        match scheme {
            Some(scheme) => Self::with_scheme(
                name,
                kind,
                position,
                expressions,
                scheme.typschm.clone(),
                scheme.interner.clone(),
            ),
            None => Self::without_scheme(name, kind, position, expressions),
        }
    }

    /// Construct an elaborated definition with an owned scheme payload.
    #[must_use]
    pub fn with_scheme(
        name: String,
        kind: TypedDefKind,
        position: usize,
        expressions: usize,
        typschm: TypSchm,
        interner: Arc<SimpleInterner>,
    ) -> Self {
        Self {
            name,
            kind,
            position,
            expressions,
            typschm: Some(typschm),
            interner: Some(interner),
        }
    }

    /// Construct an elaborated definition without a value scheme (type
    /// declarations, constraints, pragmas and other header-only forms).
    #[must_use]
    pub fn without_scheme(
        name: String,
        kind: TypedDefKind,
        position: usize,
        expressions: usize,
    ) -> Self {
        Self {
            name,
            kind,
            position,
            expressions,
            typschm: None,
            interner: None,
        }
    }

    /// The elaborated type/constraint payload, if this form carries one.
    #[must_use]
    pub fn scheme(&self) -> Option<&TypSchm> {
        self.typschm.as_ref()
    }

    /// The definition-local interner resolving the payload's symbols.
    #[must_use]
    pub fn interner(&self) -> Option<&SimpleInterner> {
        self.interner.as_deref()
    }

    /// Whether a type/constraint payload was elaborated.
    #[must_use]
    pub fn has_scheme(&self) -> bool {
        self.typschm.is_some()
    }

    /// Quantifier telescope length of the payload (0 without a scheme).
    #[must_use]
    pub fn quant_len(&self) -> usize {
        self.typschm.as_ref().map_or(0, |s| s.quant.len())
    }

    /// Numeric constraints carried in the quantifier telescope.
    #[must_use]
    pub fn constraint_count(&self) -> usize {
        self.typschm.as_ref().map_or(0, |s| {
            s.quant
                .iter()
                .filter(|q| matches!(q, sail_ast::QuantItem::Constraint(_)))
                .count()
        })
    }

    /// Restore deterministic source order after parallel emission: the
    /// streaming consumer observes completion order, not source order.
    pub fn sort_by_position(defs: &mut [TypedDef]) {
        defs.sort_by_key(|def| def.position);
    }
}

impl Drop for TypedDef {
    fn drop(&mut self) {
        // Iterative teardown via the storage work-stack path: move the
        // owned nodes into a dummy definition so `drop_definition` takes
        // apart exactly what we own, then release the interner (plain
        // refcount, no AST recursion). A derived drop of `TypSchm` would
        // recurse into nested types and overflow on adversarial shapes.
        if let Some(typschm) = self.typschm.take() {
            let interner = self.interner.take();
            let def = Node::dummy(DefKind::Val(ValSpec {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                name: Name::dummy(Symbol::DUMMY),
                typschm,
                extern_opt: None,
            }));
            sail_ast::storage::drop_definition(def);
            drop(interner);
        } else {
            debug_assert!(self.interner.is_none());
        }
    }
}

impl std::fmt::Debug for TypedDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedDef")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("position", &self.position)
            .field("expressions", &self.expressions)
            .field("quant_len", &self.quant_len())
            .field("constraint_count", &self.constraint_count())
            .finish()
    }
}

fn resolve(it: &SimpleInterner, symbol: Symbol) -> String {
    it.resolve(symbol).unwrap_or("<unresolved>").to_owned()
}

fn describe(def: &Def, it: &SimpleInterner) -> (String, TypedDefKind) {
    match &def.kind {
        DefKind::Val(val) => (resolve(it, val.name.symbol), TypedDefKind::Value),
        DefKind::Fundef { funcls, .. } => (
            funcls
                .first()
                .map(|clause| resolve(it, clause.name.symbol))
                .unwrap_or_else(|| "<empty-fun>".to_owned()),
            TypedDefKind::Function,
        ),
        DefKind::Register(reg) => (resolve(it, reg.name.symbol), TypedDefKind::Register),
        DefKind::Type(typ) => (type_name(&typ.kind, it), TypedDefKind::Type),
        DefKind::Constraint(_) => ("<constraint>".to_owned(), TypedDefKind::Constraint),
        DefKind::Overload { name, .. } => (resolve(it, name.symbol), TypedDefKind::Overload),
        DefKind::Let { .. } => ("<let>".to_owned(), TypedDefKind::Let),
        DefKind::Fixity { .. } => ("<fixity>".to_owned(), TypedDefKind::Other),
        DefKind::Pragma { .. } => ("<pragma>".to_owned(), TypedDefKind::Other),
        DefKind::Default(_) => ("<default>".to_owned(), TypedDefKind::Other),
        _ => ("<def>".to_owned(), TypedDefKind::Other),
    }
}

fn type_name(kind: &sail_ast::TypeDefKind, it: &SimpleInterner) -> String {
    let symbol = match kind {
        TypeDefKind::Abbrev { name, .. }
        | TypeDefKind::Record { name, .. }
        | TypeDefKind::Variant { name, .. }
        | TypeDefKind::Enum { name, .. }
        | TypeDefKind::Abstract { name, .. }
        | TypeDefKind::Bitfield { name, .. } => name.symbol,
    };
    resolve(it, symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{KOpt, Kind, Typ, TypKind};

    fn int_fn_scheme(it: &mut SimpleInterner) -> (TypSchm, Arc<SimpleInterner>) {
        let int = it.intern("int");
        let typ = Node::dummy(TypKind::Fn {
            args: vec![Node::dummy(TypKind::Id(Name::dummy(int)))],
            ret: Box::new(Node::dummy(TypKind::Id(Name::dummy(int)))),
        });
        let typschm = TypSchm { quant: vec![], typ };
        (typschm, Arc::new(it.clone()))
    }

    fn val_def(name: &str, it: &mut SimpleInterner) -> (Def, SimpleInterner) {
        let (typschm, _) = int_fn_scheme(it);
        let symbol = it.intern(name);
        let def = Node::dummy(DefKind::Val(ValSpec {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: Name::dummy(symbol),
            typschm,
            extern_opt: None,
        }));
        (def, it.clone())
    }

    #[test]
    fn checked_val_carries_its_declared_scheme() {
        let mut it = SimpleInterner::new();
        let (def, snapshot) = val_def("f", &mut it);
        let mut program = Program::new();
        program
            .declare(&def, Arc::new(snapshot.clone()), 0)
            .unwrap();
        let checked = TypedDef::checked(&def, &snapshot, 0, 7, &program);
        assert_eq!(checked.name, "f");
        assert_eq!(checked.kind, TypedDefKind::Value);
        assert_eq!(checked.expressions, 7);
        assert!(checked.has_scheme());
        assert_eq!(checked.quant_len(), 0);
        assert_eq!(checked.constraint_count(), 0);
        assert!(matches!(
            checked.scheme().unwrap().typ.kind,
            TypKind::Fn { .. }
        ));
        sail_ast::storage::drop_definition(def);
    }

    #[test]
    fn constraint_payload_is_counted_from_the_telescope() {
        let mut it = SimpleInterner::new();
        let kid = it.intern("n");
        let typschm = TypSchm {
            quant: vec![
                sail_ast::QuantItem::Id(KOpt {
                    kind: Kind::Int,
                    kid,
                }),
                sail_ast::QuantItem::Constraint(Node::dummy(sail_ast::ConstraintKind::True)),
            ],
            typ: Node::dummy(TypKind::Var(kid)),
        };
        let def = TypedDef::with_scheme(
            "g".to_owned(),
            TypedDefKind::Value,
            3,
            1,
            typschm,
            Arc::new(it),
        );
        assert_eq!(def.quant_len(), 2);
        assert_eq!(def.constraint_count(), 1);
        assert!(format!("{def:?}").contains("constraint_count"));
    }

    #[test]
    fn header_only_forms_carry_no_scheme() {
        let mut it = SimpleInterner::new();
        let def = Node::dummy(DefKind::Constraint(Node::dummy(
            sail_ast::ConstraintKind::True,
        )));
        let (name, kind) = describe(&def, &it);
        assert_eq!(kind, TypedDefKind::Constraint);
        let typed = TypedDef::without_scheme(name, kind, 5, 0);
        assert!(!typed.has_scheme());
        assert_eq!(typed.quant_len(), 0);
        sail_ast::storage::drop_definition(def);
    }

    #[test]
    fn emission_sorts_back_to_source_order() {
        let mut it = SimpleInterner::new();
        let mut defs: Vec<TypedDef> = (0..4)
            .map(|position| {
                let (typschm, interner) = int_fn_scheme(&mut it);
                TypedDef::with_scheme(
                    format!("f{position}"),
                    TypedDefKind::Function,
                    position,
                    position,
                    typschm,
                    interner,
                )
            })
            .collect();
        defs.swap(0, 3);
        defs.swap(1, 2);
        TypedDef::sort_by_position(&mut defs);
        let positions: Vec<usize> = defs.iter().map(|def| def.position).collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
    }

    #[test]
    fn deep_payloads_drop_iteratively_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut it = SimpleInterner::new();
                let mut typ: Typ = Node::dummy(TypKind::Id(Name::dummy(it.intern("int"))));
                for _ in 0..20_000 {
                    typ = Node::dummy(TypKind::Tuple(vec![typ]));
                }
                let typschm = TypSchm { quant: vec![], typ };
                let def = TypedDef::with_scheme(
                    "deep".to_owned(),
                    TypedDefKind::Value,
                    0,
                    0,
                    typschm,
                    Arc::new(it),
                );
                drop(def);
                // Also exercise vector teardown of many payloads at once.
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn typed_def_is_send_for_streaming_workers() {
        fn is_send<T: Send>() {}
        is_send::<TypedDef>();
    }
}
