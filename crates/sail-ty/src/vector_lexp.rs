#![forbid(unsafe_code)]
//! Vector-LHS-Verdrahtung (Lane 30, exklusiv neu).
//!
//! Adapter `LE_vector` / `LE_vector_range` (`sail_ast::LExpKind::Vector` /
//! `VectorRange`) auf die Lane-07-Pruefung
//! (`crate::lexp_bounds::check_vector_index` / `check_vector_slice`):
//!
//! - `LE_vector (v_lexp, index)`: Wurzel muss `mutable`-Local oder `Register`
//!   sein ([`check_writable`], OCaml `Env.lookup_id` in `infer_lexp`),
//!   Synonyme sind vom Aufrufer expandiert (`resolve`), dann
//!   [`vector_root_of_typ`] (`vector('len,'elem)` / `bitvector('len)`,
//!   sonst `"Invalid vector assignment l-expression"`), Indexausdruck
//!   vollstaendig per `infer_exp` + `bind_numeric` (`int`-artig, sonst
//!   Mismatch), danach `0 <= index /\ index < len` beweisen; Ergebnis ist der
//!   Element-Typ (`'elem` bzw. `bit`).
//! - `LE_vector_range (v_lexp, e1, e2)`: Wurzel muss nach Expansion
//!   `bitvector('len)` sein (sonst Mismatch, OCaml
//!   `"Cannot assign slice of non vector type"` via
//!   [`crate::lexp_bounds::check_slice_root_len`]), beide Grenzen
//!   vollstaendig pruefen, dann ordnungsabhaengig
//!   (`Env.get_default_order`, kein impliziter Fallback):
//!   `inc`: `slice_len = (e2 - e1) + 1`, `0 <= e1 /\ e1 <= e2 /\ e2 < len`;
//!   `dec`: `slice_len = (e1 - e2) + 1`, `0 <= e2 /\ e2 <= e1 /\ e1 < len`;
//!   Ergebnis ist `bitvector(slice_len)`.
//! - `e1`/`e2` sind positionsgebunden (`v[e1 .. e2]`, AST `lo` = erste,
//!   `hi` = zweite Grenze); `lo`/`hi` sind keine Min/Max-Namen.
//! - Bitfield-Wurzel (`E_id`-Feld): kein Bounds-Check hier, Delegation an den
//!   `bitfield`-Slice ausserhalb dieser Lane ([`as_vector`]/[`as_vector_range`]
//!   geben `None` fuer `Field`/`Other` zurueck).
//! - RHS darf Index-Fehler nicht umgehen: [`check_assign`] prueft die
//!   LExp-Seite zuerst (LExp-Fehler gewinnt immer).
//!
//! Beweisverpflichtungen laufen ausschliesslich ueber den
//! [`Solver`](crate::solver::Solver)-Trait (`Z-TraitOnly`, `Z-UnknownSound`);
//! Tests entscheiden mit [`StubSolver`](crate::solver::StubSolver).
//!
//! Insert-Spec fuer den Main-Agent (`canonical_body.rs`, Hook-Insertion):
//! - Anker: [`INFER_LEXP_ANCHOR`] (`infer_lexp`; heute `assignment_type` +
//!   `Assign`-Arm, die Vektor-Ziele noch mit `Unsupported` ablehnen).
//! - Exakte Calls:
//!   [`HOOK_CALL_INDEX`], [`HOOK_CALL_SLICE`], [`HOOK_CALL_ASSIGN`].
//!   Reihenfolge ist load-bearing: Wurzel expandieren, Indexausdruecke
//!   inferieren, Default-Order holen, dann `infer_vector_elem` /
//!   `infer_vector_slice` zuerst, erst danach RHS gegen den hier bestimmten
//!   Element-/Slice-Typ checken, Fehler via `check_assign` priorisieren.

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::lexp_bounds::{
    RootBinding, VectorRoot, check_assign as lexp_check_assign,
    check_root as lexp_check_root, check_vector_index as lexp_check_index,
    check_vector_slice as lexp_check_slice,
};
use crate::nexp::{MiniOrder, MiniTyp, Nexp};
use crate::solver::Solver;
use sail_ast::{Exp, Interner, LExp, LExpKind, Order as AstOrder, SimpleInterner, Symbol, Typ, TypArgKind, TypKind};

/// Ankername fuer die Main-Agent-Hook-Insertion in `canonical_body.rs`.
pub const INFER_LEXP_ANCHOR: &str = "infer_lexp";

/// Exakter Call fuer `LE_vector` am [`INFER_LEXP_ANCHOR`]-Anker.
pub const HOOK_CALL_INDEX: &str =
    "crate::vector_lexp::infer_vector_elem(solver, &root, &elem, &index, &index_typ, span)";

/// Exakter Call fuer `LE_vector_range` am [`INFER_LEXP_ANCHOR`]-Anker.
pub const HOOK_CALL_SLICE: &str =
    "crate::vector_lexp::infer_vector_slice(solver, &root, &first, &first_typ, &second, &second_typ, order, span)";

/// Exakter Call fuer die Assign-Prioritaet (LExp-Fehler gewinnt).
pub const HOOK_CALL_ASSIGN: &str = "crate::vector_lexp::check_assign(lexp, rhs, span)";

/// `sail_ast::Order` -> [`MiniOrder`] (reine Positionsabbildung, kein Fallback).
#[must_use]
pub fn ast_order_to_mini(order: &AstOrder) -> MiniOrder {
    match order {
        AstOrder::Inc => MiniOrder::Inc,
        AstOrder::Dec => MiniOrder::Dec,
    }
}

/// Zerlegt `LExpKind::Vector { lexp, index }` (`l[i]`).
/// Gibt `(Basis, Indexausdruck)` zurueck, sonst `None`
/// (u.a. `Field`/Bitfield-Delegation, `Tuple`, `Id`).
#[must_use]
pub fn as_vector(lexp: &LExp) -> Option<(&LExp, &Exp)> {
    match &lexp.kind {
        LExpKind::Vector { lexp: base, index } => Some((base, index)),
        _ => None,
    }
}

/// Zerlegt `LExpKind::VectorRange { lexp, lo, hi }` (`v[e1 .. e2]`).
/// Gibt `(Basis, erste Grenze, zweite Grenze)` zurueck — `lo` ist die erste,
/// `hi` die zweite geschriebene Grenze (keine Min/Max-Semantik; die Ordnung
/// aus [`ast_order_to_mini`] legt fest, welche Seite unten liegt).
#[must_use]
pub fn as_vector_range(lexp: &LExp) -> Option<(&LExp, &Exp, &Exp)> {
    match &lexp.kind {
        LExpKind::VectorRange { lexp: base, lo, hi } => Some((base, lo, hi)),
        _ => None,
    }
}


/// Vereinte Namenssicht über mehrere Interner-Tabellen.
///
/// Symbole gelten nur in ihrer eigenen Tabelle; ein kanonischer Wurzeltyp
/// kann Köpfe aus der Root-Tabelle und Binder aus Scope-/Checker-Tabellen
/// mischen (rigide `with`-Binder leben nicht in der Root-Tabelle). Die
/// Auflösung läuft lokal, dann Kette; frische Namen (Mini-Bau) landen lokal.
/// Wie der restliche kanonische Pfad wird Identität über Textnamen
/// verglichen (rigide Frische pro Scope trennt Binder gleichen Namens).
pub struct Union<'a> {
    local: SimpleInterner,
    chain: Vec<&'a dyn Interner>,
}

impl<'a> Union<'a> {
    /// Neue Sicht über `chain` (erste Tabelle gewinnt bei Konflikten).
    pub fn over(chain: Vec<&'a dyn Interner>) -> Self {
        Self {
            local: SimpleInterner::new(),
            chain,
        }
    }
}

impl Interner for Union<'_> {
    fn intern(&mut self, text: &str) -> Symbol {
        self.local.intern(text)
    }
    fn resolve(&self, symbol: Symbol) -> Option<&str> {
        if let r @ Some(_) = self.local.resolve(symbol) {
            return r;
        }
        for (i, it) in self.chain.iter().enumerate() {
            if let r @ Some(text) = it.resolve(symbol) {
                return r;
            }
        }
        None
    }
    fn len(&self) -> usize {
        self.local.len()
    }
    fn is_empty(&self) -> bool {
        self.local.is_empty()
    }
}

fn invalid_vector_assignment(span: Span) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported("Invalid vector assignment l-expression".to_owned()),
    }
}

/// Extrahiert die expandierte Vektor-Wurzel aus einem kanonischen `Typ`.
///
/// Der Aufrufer uebergibt den bereits per `resolve` (Synonym-Expansion)
/// aufgeloesten Typ:
/// - `vector(len, elem)`: `App` mit 2 Args (`NExp` + `Typ`) ->
///   [`VectorRoot::Vector`].
/// - `bitvector(len)`: `App` mit 1 Arg (`NExp`) -> [`VectorRoot::Bitvector`].
/// - Sonst: `"Invalid vector assignment l-expression"` (kein Bounds-Beweis,
///   kein fabrizierter Typ).
pub fn vector_root_of_typ(typ: &Typ, union: &mut Union<'_>, span: Span) -> TypeResult<VectorRoot> {
    root_in(typ, union, span)
}

fn root_in(typ: &Typ, interner: &mut impl Interner, span: Span) -> TypeResult<VectorRoot> {
    match &typ.kind {
        TypKind::App { name, args } => {
            let text = interner
                .resolve(name.symbol)
                .ok_or_else(|| invalid_vector_assignment(span))?;
            match (text, args.as_slice()) {
                ("vector", [len_arg, _elem_arg]) => {
                    let TypArgKind::NExp(len_node) = &len_arg.kind else {
                        return Err(invalid_vector_assignment(span));
                    };
                    let len = crate::conv::nexp_from_ast(len_node, &mut *interner)
                        .ok_or_else(|| invalid_vector_assignment(span))?;
                    // Kein elem-Conv hier: die rohe Ansicht traegt die
                    // unsubstituierte Variable; das Element liefert der
                    // Aufrufer substituiert (siehe check_vector_index).
                    Ok(VectorRoot::Vector { len })
                }
                ("bitvector", [len_arg]) => {
                    let TypArgKind::NExp(len_node) = &len_arg.kind else {
                        return Err(invalid_vector_assignment(span));
                    };
                    let len = crate::conv::nexp_from_ast(len_node, &mut *interner)
                        .ok_or_else(|| invalid_vector_assignment(span))?;
                    Ok(VectorRoot::Bitvector { len })
                }
                _ => Err(invalid_vector_assignment(span)),
            }
        }
        _ => Err(invalid_vector_assignment(span)),
    }
}

/// Prueft, dass die `LE_id`-Wurzel beschreibbar ist (mutable-Local oder
/// Register) und gibt die expandierte [`VectorRoot`] zurueck.
pub fn check_writable(
    name: &str,
    binding: &RootBinding,
    span: Span,
) -> TypeResult<VectorRoot> {
    lexp_check_root(name, binding, span)
}

/// Vollstaendige `LE_vector`-Pruefung: Index-Typ (`int`-artig) zuerst, dann
/// `0 <= index /\ index < len` beweisen, Element-Typ liefern.
/// Delegiert an `lexp_bounds::check_vector_index`.
pub fn infer_vector_elem<S: Solver>(
    solver: &S,
    root: &VectorRoot,
    elem: &MiniTyp,
    index: &Nexp,
    index_typ: &MiniTyp,
    span: Span,
) -> TypeResult<MiniTyp> {
    lexp_check_index(solver, root, elem, index, index_typ, span)
}

/// Vollstaendige `LE_vector_range`-Pruefung: beide Index-Typen zuerst, dann
/// ordnungsabhaengige Bounds beweisen, Slice-Typ liefern.
/// `first`/`second` sind erste/zweite geschriebene Grenze, `order` kommt aus
/// [`ast_order_to_mini`] (`Env.get_default_order`, kein Fallback).
/// Delegiert an `lexp_bounds::check_vector_slice`.
pub fn infer_vector_slice<S: Solver>(
    solver: &S,
    root: &VectorRoot,
    first: &Nexp,
    first_typ: &MiniTyp,
    second: &Nexp,
    second_typ: &MiniTyp,
    order: MiniOrder,
    span: Span,
) -> TypeResult<MiniTyp> {
    lexp_check_slice(
        solver, root, first, first_typ, second, second_typ, order, span,
    )
}

/// Assign-Reihenfolge: LExp-Seite zuerst; ihr Fehler gewinnt immer.
/// Delegiert an `lexp_bounds::check_assign`.
pub fn check_assign<T>(
    lexp: TypeResult<T>,
    rhs: TypeResult<()>,
    span: Span,
) -> TypeResult<T> {
    lexp_check_assign(lexp, rhs, span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TypeErrorKind;
    use crate::solver::StubSolver;
    use sail_ast::{Name, Node, NExpKind, TypArg};

    fn solver() -> StubSolver {
        StubSolver::new()
    }

    fn mk_name(it: &mut impl Interner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn mk_nexp_const(it: &mut impl Interner, value: &str) -> sail_ast::NExp {
        let _ = it;
        Node::dummy(NExpKind::Constant(value.to_owned()))
    }

    fn mk_vector_typ(
        it: &mut impl Interner,
        len: &str,
        elem: Typ,
    ) -> Typ {
        let name = mk_name(it, "vector");
        let len_arg: TypArg = Node::dummy(TypArgKind::NExp(mk_nexp_const(it, len)));
        let elem_arg: TypArg = Node::dummy(TypArgKind::Typ(elem));
        Node::dummy(TypKind::App {
            name,
            args: vec![len_arg, elem_arg],
        })
    }

    fn mk_bitvector_typ(it: &mut impl Interner, len: &str) -> Typ {
        let name = mk_name(it, "bitvector");
        let len_arg: TypArg = Node::dummy(TypArgKind::NExp(mk_nexp_const(it, len)));
        Node::dummy(TypKind::App {
            name,
            args: vec![len_arg],
        })
    }

    fn mk_int_typ(it: &mut impl Interner) -> Typ {
        Node::dummy(TypKind::Id(mk_name(it, "int")))
    }

    fn mk_id_lexp(it: &mut impl Interner, name: &str) -> LExp {
        Node::dummy(LExpKind::Id(mk_name(it, name)))
    }

    fn mk_id_exp(it: &mut impl Interner, name: &str) -> Exp {
        Node::dummy(sail_ast::ExpKind::Id(mk_name(it, name)))
    }

    #[test]
    fn hook_spec_names_anchor_and_calls() {
        assert_eq!(INFER_LEXP_ANCHOR, "infer_lexp");
        assert!(HOOK_CALL_INDEX.contains("infer_vector_elem"));
        assert!(HOOK_CALL_SLICE.contains("infer_vector_slice"));
        assert!(HOOK_CALL_ASSIGN.contains("check_assign"));
    }

    #[test]
    fn order_mapping_is_exact() {
        assert_eq!(ast_order_to_mini(&AstOrder::Inc), MiniOrder::Inc);
        assert_eq!(ast_order_to_mini(&AstOrder::Dec), MiniOrder::Dec);
    }

    #[test]
    fn lexp_shape_splits_vector_and_range() {
        let mut it = sail_ast::SimpleInterner::new();
        let base = mk_id_lexp(&mut it, "v");
        let idx = mk_id_exp(&mut it, "i");
        let lexp: LExp = Node::dummy(LExpKind::Vector {
            lexp: Box::new(base.clone()),
            index: Box::new(idx.clone()),
        });
        assert!(as_vector(&lexp).is_some());
        assert!(as_vector_range(&lexp).is_none());

        let lo = mk_id_exp(&mut it, "a");
        let hi = mk_id_exp(&mut it, "b");
        let range: LExp = Node::dummy(LExpKind::VectorRange {
            lexp: Box::new(base.clone()),
            lo: Box::new(lo),
            hi: Box::new(hi),
        });
        let (rbase, first, second) = as_vector_range(&range).expect("range splits");
        assert!(as_vector(&range).is_none());
        let _ = (rbase, first, second);

        let id = mk_id_lexp(&mut it, "x");
        assert!(as_vector(&id).is_none());
        assert!(as_vector_range(&id).is_none());

        // Bitfield-Delegation: Field faellt durch (None), kein Bounds-Check hier.
        let field: LExp = Node::dummy(LExpKind::Field {
            lexp: Box::new(base),
            field: mk_name(&mut it, "f"),
        });
        assert!(as_vector(&field).is_none());
        assert!(as_vector_range(&field).is_none());
    }

    #[test]
    fn root_extraction_vector_and_bitvector() {
        let mut it = sail_ast::SimpleInterner::new();
        let elem = mk_int_typ(&mut it);
        let vec = mk_vector_typ(&mut it, "8", elem);
        let root = { let mut union = Union::over(vec![&it as &dyn sail_ast::Interner]); vector_root_of_typ(&vec, &mut union, Span::DUMMY) }.unwrap();
        match root {
            VectorRoot::Vector { len } => {
                assert_eq!(len, Nexp::Const(8));
            }
            other => panic!("expected Vector root, got {other:?}"),
        }

        let bv = mk_bitvector_typ(&mut it, "4");
        let root = { let mut union = Union::over(vec![&it as &dyn sail_ast::Interner]); vector_root_of_typ(&bv, &mut union, Span::DUMMY) }.unwrap();
        assert_eq!(root, VectorRoot::Bitvector { len: Nexp::Const(4) });
    }

    #[test]
    fn root_extraction_rejects_non_vector() {
        let mut it = sail_ast::SimpleInterner::new();
        let int = mk_int_typ(&mut it);
        let err = { let mut union = Union::over(vec![&it as &dyn sail_ast::Interner]); vector_root_of_typ(&int, &mut union, Span::DUMMY) }.unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::Unsupported(ref msg) if msg == "Invalid vector assignment l-expression"),
            "unexpected {err:?}"
        );

        // `bits(4)` ist kein beschreibbarer Vektor-Root (Slice braucht bitvector).
        let bits: Typ = crate::conv::typ_to_ast(&MiniTyp::Bits(Box::new(Nexp::Const(4))), &mut it);
        let err = { let mut union = Union::over(vec![&it as &dyn sail_ast::Interner]); vector_root_of_typ(&bits, &mut union, Span::DUMMY) }.unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn index_proves_and_yields_element_typ() {
        let root = VectorRoot::Vector { len: Nexp::Const(8) };
        let got = infer_vector_elem(&solver(), &root, &MiniTyp::Bool, &Nexp::Const(3), &MiniTyp::Int, Span::DUMMY).unwrap();
        assert_eq!(got, MiniTyp::Bool);

        let bv = VectorRoot::Bitvector { len: Nexp::Const(4) };
        let got = infer_vector_elem(&solver(), &bv, &MiniTyp::Bit, &Nexp::Const(0), &MiniTyp::Int, Span::DUMMY).unwrap();
        assert_eq!(got, MiniTyp::Bit);
    }

    #[test]
    fn index_out_of_bounds_is_constraint_not_proved() {
        let root = VectorRoot::Vector { len: Nexp::Const(8) };
        let err = infer_vector_elem(&solver(), &root, &MiniTyp::Int, &Nexp::Const(8), &MiniTyp::Int, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)));
    }

    #[test]
    fn slice_is_order_dependent_through_adapter() {
        let root = VectorRoot::Bitvector { len: Nexp::Const(8) };
        // inc: v[2..5] ok -> len 4
        let got = infer_vector_slice(
            &solver(),
            &root,
            &Nexp::Const(2),
            &MiniTyp::Int,
            &Nexp::Const(5),
            &MiniTyp::Int,
            ast_order_to_mini(&AstOrder::Inc),
            Span::DUMMY,
        )
        .unwrap();
        let MiniTyp::Bits(len) = &got else {
            panic!("expected Bits, got {got:?}");
        };
        assert_eq!(len.as_const(), Some(4));

        // dec mit denselben Grenzen scheitert (5<=2 falsch).
        let err = infer_vector_slice(
            &solver(),
            &root,
            &Nexp::Const(2),
            &MiniTyp::Int,
            &Nexp::Const(5),
            &MiniTyp::Int,
            ast_order_to_mini(&AstOrder::Dec),
            Span::DUMMY,
        )
        .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)));

        // dec: v[5..2] ok -> len 4.
        let got = infer_vector_slice(
            &solver(),
            &root,
            &Nexp::Const(5),
            &MiniTyp::Int,
            &Nexp::Const(2),
            &MiniTyp::Int,
            ast_order_to_mini(&AstOrder::Dec),
            Span::DUMMY,
        )
        .unwrap();
        let MiniTyp::Bits(len) = &got else {
            panic!("expected Bits, got {got:?}");
        };
        assert_eq!(len.as_const(), Some(4));
    }

    #[test]
    fn slice_on_vector_root_is_mismatch() {
        let root = VectorRoot::Vector { len: Nexp::Const(8) };
        let err = infer_vector_slice(
            &solver(),
            &root,
            &Nexp::Const(0),
            &MiniTyp::Int,
            &Nexp::Const(1),
            &MiniTyp::Int,
            MiniOrder::Inc,
            Span::DUMMY,
        )
        .unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
    }

    #[test]
    fn writable_roots_pass_immutable_unbound_rejected() {
        let root = VectorRoot::Bitvector { len: Nexp::Const(8) };
        assert_eq!(
            check_writable("v", &RootBinding::Mutable(root.clone()), Span::DUMMY).unwrap(),
            root
        );
        assert_eq!(
            check_writable("r", &RootBinding::Register(root.clone()), Span::DUMMY).unwrap(),
            root
        );
        let err = check_writable("x", &RootBinding::Immutable, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::RegisterError(_)));
        let err = check_writable("fresh", &RootBinding::Unbound, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::UnboundVar(_)));
    }

    #[test]
    fn non_int_index_rejected_before_proof() {
        let root = VectorRoot::Vector { len: Nexp::Const(8) };
        let err = infer_vector_elem(&solver(), &root, &MiniTyp::Int, &Nexp::Const(0), &MiniTyp::Bool, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
    }

    #[test]
    fn assign_prioritises_lexp_errors() {
        let lexp: TypeResult<MiniTyp> = Err(TypeError::constraint_not_proved(
            Span::DUMMY,
            "vector index bounds: (('i >= 0) & ('i < 8))",
        ));
        let rhs: TypeResult<()> = Ok(());
        let err = check_assign(lexp, rhs, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)));
    }
}
