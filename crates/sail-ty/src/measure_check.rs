#![forbid(unsafe_code)]
//! Termination-measure typing helpers (Lane-28).
//!
//! Pure replacement for `unsupported("termination-measure typing")` in
//! `canonical_body::check`. Mirrors OCaml `check_termination_measure`
//! (`sail/src/lib/type_check.ml` ~L4829, wired into `check_fundef` for
//! `Rec_measure` ~L4960 and into `DEF_measure` ~L5616):
//!
//! - the measure pattern binds against the single argument type, or against
//!   the tuple of all argument types for multi-argument functions
//!   ([`measure_arity`] / [`expected_pat_type`]);
//! - the measure body checks `<= int` in the clause environment extended
//!   with the pattern bindings (builtin [`body_builtin()], discharged by the
//!   body pass via `self.builtin("int")`);
//! - the measured function is resolved through the declaration tables
//!   ([`lookup_function_args`], [`check_measure_target`],
//!   [`check_loop_measures_target`]). On the Lane-19 `Program` this lookup
//!   additionally guards on the `measures` / `loop_measures` presence
//!   tables (`Program::measure` / `Program::loop_measure`); on the current
//!   `Program` it resolves the `val`-spec signature via `Program::value`,
//!   which is the same signature those tables gate.
//!
//! # Insert-Spec for the Main-Agent (`canonical_body.rs`, VERBOTEN here)
//!
//! Anker: `fn function()`.
//!
//! Exakter Call (ersetzt dort NICHTS bestehendes, sondern den
//! `unsupported(def.span, "termination-measure typing")`-Fruehreturn in
//! `check()`; `function()` erhaelt das Mass als zusaetzlichen Parameter):
//! ```ignore
//! // Anker: fn function(&mut self, funcl: &Funcl) in canonical_body.rs
//! // Neu:   fn function(&mut self, funcl: &Funcl, measure: Option<(&Pat, &Exp)>)
//! if let Some((pat, exp)) = measure {
//!     let args = crate::measure_check::lookup_function_args(
//!         self.program, name, self.position, funcl.span,
//!     )?;
//!     let bound = crate::measure_check::expected_pat_type(&args, pat.span);
//!     // `bound` via `Type::owned(bound, self.it.clone())`, Scope anpassen,
//!     // dann wie OCaml `bind_pat_no_guard` + `check_exp <= int`:
//!     let mut child = Scope {
//!         locals: BTreeMap::new(),
//!         facts: Vec::new(),
//!         parent: Some(scope.clone()),
//!     };
//!     self.pattern(pat, bound_ty, false, &mut child)?;
//!     self.run(vec![Task {
//!         exp,
//!         expected: self.builtin(crate::measure_check::body_builtin()),
//!         scope: Arc::new(child),
//!         returning: result.clone(),
//!     }])?;
//! }
//! ```
//! `DefKind::Measure` nutzt [`check_measure_target`], `DefKind::LoopMeasures`
//! nutzt [`check_loop_measures_target`] (ein `Task` pro Mass mit
//! [`body_builtin()] als Erwartungstyp). In-place `ExpKind::Loop`-Masse
//! bleiben beim bestehenden `Loop`-Arm (`<= int`, OCaml `E_loop` ~L3921).

use crate::{
    error::{TypeError, TypeErrorKind, TypeResult},
    program::Program,
};
use sail_ast::{Exp, Interner, Loop, Name, Node, NodeId, SimpleInterner, Span, Typ, TypKind};

/// Which type a termination-measure pattern binds against.
///
/// Mirrors `check_termination_measure`: `match arg_typs with [x] -> x | _ ->
/// tuple`. Companion to the body-pass plan; kept separate so tests pin the
/// rule without definition-level inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureArity {
    Single,
    Tuple,
}

/// Arity of the measured function: exactly one argument binds directly,
/// any other count (including zero) binds against the tuple.
#[must_use]
pub fn measure_arity(arg_count: usize) -> MeasureArity {
    if arg_count == 1 {
        MeasureArity::Single
    } else {
        MeasureArity::Tuple
    }
}

/// Expected pattern type for a measure: the single argument type, or the
/// tuple of all argument types for multi-argument functions.
#[must_use]
pub fn expected_pat_type(arg_typs: &[Typ], span: Span) -> Typ {
    match arg_typs {
        [single] => single.clone(),
        _ => Node::new(NodeId::DUMMY, span, TypKind::Tuple(arg_typs.to_vec())),
    }
}

/// Builtin the measure body checks against (`<= int`, OCaml `int_typ`).
#[must_use]
pub fn body_builtin() -> &'static str {
    "int"
}

fn resolve(it: &SimpleInterner, name: &Name) -> TypeResult<String> {
    it.resolve(name.symbol)
        .map(str::to_owned)
        .ok_or_else(|| TypeError::unbound_var(name.span, format!("symbol {}", name.symbol)))
}

/// Argument types of the measured function at `position`.
///
/// This is the `measures` / `loop_measures` table lookup: the function must
/// have a `val`-spec arrow signature (`Typ_fn`), otherwise OCaml reports
/// "Function val spec is not a function type". Returns the cloned argument
/// types so the caller can build [`expected_pat_type`].
pub fn lookup_function_args(
    program: &Program,
    name: &str,
    position: usize,
    span: Span,
) -> TypeResult<Vec<Typ>> {
    let scheme = program
        .value(name, position)
        .ok_or_else(|| TypeError::unbound_var(span, name.to_owned()))?;
    match &scheme.typschm.typ.kind {
        TypKind::Fn { args, .. } => Ok(args.clone()),
        _ => Err(TypeError {
            span,
            kind: TypeErrorKind::QuantError(format!(
                "function '{name}' does not have a function type"
            )),
        }),
    }
}

/// Validate a `DefKind::Measure` target and return its argument types.
///
/// Resolves `name` via `it`, then delegates to [`lookup_function_args`].
/// The caller binds `pat` against [`expected_pat_type`] of the result and
/// checks `exp` against [`body_builtin()].
pub fn check_measure_target(
    program: &Program,
    name: &Name,
    it: &SimpleInterner,
    position: usize,
    span: Span,
) -> TypeResult<Vec<Typ>> {
    let target = resolve(it, name)?;
    lookup_function_args(program, &target, position, span)
}

/// Validate a `DefKind::LoopMeasures` target and return the measure count.
///
/// Each `(Loop, Exp)` body checks `<= int` in the function scope (OCaml
/// `move_loop_measures` rewrite: `check_exp env exp int_typ`). The loop
/// kind itself needs no typing.
pub fn check_loop_measures_target(
    program: &Program,
    name: &Name,
    it: &SimpleInterner,
    measures: &[(Loop, Exp)],
    position: usize,
    span: Span,
) -> TypeResult<usize> {
    let target = resolve(it, name)?;
    lookup_function_args(program, &target, position, span)?;
    Ok(measures.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{DefKind, Span};
    use sail_parse::{ParserContext, RealParser};
    use std::sync::Arc;

    fn parse(sources: &[&str]) -> Vec<(sail_ast::Def, Arc<SimpleInterner>)> {
        let mut context = ParserContext::default();
        sources
            .iter()
            .map(|source| {
                let mut parser = RealParser::with_context(context.clone());
                let def = parser.parse_def(source, Span::DUMMY).unwrap();
                context = parser.context();
                (def, Arc::new(parser.into_interner()))
            })
            .collect()
    }

    fn declare(defs: &[(sail_ast::Def, Arc<SimpleInterner>)]) -> Program {
        let mut program = Program::new();
        for (position, (def, it)) in defs.iter().enumerate() {
            program.declare(def, it.clone(), position).unwrap();
        }
        program
    }

    #[test]
    fn arity_is_single_only_for_one_argument() {
        assert_eq!(measure_arity(1), MeasureArity::Single);
        assert_eq!(measure_arity(0), MeasureArity::Tuple);
        assert_eq!(measure_arity(2), MeasureArity::Tuple);
        assert_eq!(measure_arity(3), MeasureArity::Tuple);
    }

    #[test]
    fn single_arg_expected_type_clones_without_tuple() {
        let defs = parse(&["val f : int -> int"]);
        let program = declare(&defs);
        let args = lookup_function_args(&program, "f", 0, Span::DUMMY).unwrap();
        assert_eq!(args.len(), 1);
        assert_eq!(measure_arity(args.len()), MeasureArity::Single);
        let expected = expected_pat_type(&args, Span::DUMMY);
        assert_eq!(expected, args[0]);
        assert!(!matches!(expected.kind, TypKind::Tuple(_)));
    }

    #[test]
    fn multi_arg_expected_type_is_tuple_in_order() {
        let defs = parse(&["val g : (int, bool) -> int"]);
        let program = declare(&defs);
        let args = lookup_function_args(&program, "g", 0, Span::DUMMY).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(measure_arity(args.len()), MeasureArity::Tuple);
        let expected = expected_pat_type(&args, Span::DUMMY);
        match &expected.kind {
            TypKind::Tuple(items) => assert_eq!(items, &args),
            other => panic!("expected tuple, got {other:?}"),
        }
    }

    #[test]
    fn missing_function_is_unbound() {
        let program = Program::new();
        let error = lookup_function_args(&program, "absent", 0, Span::DUMMY).unwrap_err();
        assert!(matches!(error.kind, TypeErrorKind::UnboundVar(_)));
    }

    #[test]
    fn non_function_value_is_rejected() {
        let defs = parse(&["enum E = A | B"]);
        let program = declare(&defs);
        let error = lookup_function_args(&program, "A", 0, Span::DUMMY).unwrap_err();
        assert!(
            matches!(error.kind, TypeErrorKind::QuantError(_)),
            "unexpected {error:?}"
        );
    }

    #[test]
    fn measure_decl_resolves_single_arg_target() {
        let defs = parse(&["val f : int -> int", "termination_measure f(x) = x"]);
        let program = declare(&defs);
        let (_, it) = &defs[1];
        let DefKind::Measure { name, pat, exp } = &defs[1].0.kind else {
            panic!("expected Measure, got {:?}", defs[1].0.kind)
        };
        let args = check_measure_target(&program, name, it, 0, defs[1].0.span).unwrap();
        assert_eq!(measure_arity(args.len()), MeasureArity::Single);
        let expected = expected_pat_type(&args, pat.span);
        assert_eq!(expected, args[0]);
        assert_eq!(body_builtin(), "int");
        let _ = exp;
    }

    #[test]
    fn loop_measures_decl_resolves_target_and_count() {
        let defs = parse(&[
            "val test : int -> int",
            "termination_measure test while 5",
        ]);
        let program = declare(&defs);
        let (_, it) = &defs[1];
        let DefKind::LoopMeasures { name, measures } = &defs[1].0.kind else {
            panic!("expected LoopMeasures, got {:?}", defs[1].0.kind)
        };
        assert_eq!(measures.len(), 1);
        assert!(matches!(measures[0].0, Loop::While));
        let count =
            check_loop_measures_target(&program, name, it, measures, 0, defs[1].0.span).unwrap();
        assert_eq!(count, 1);
        assert_eq!(body_builtin(), "int");
    }

    #[test]
    fn inline_rec_measure_parses_and_resolves() {
        let defs = parse(&[
            "val f : nat -> nat",
            "function {x => x} f(x) = if x == 0 then 0 else f(x - 1)",
        ]);
        let program = declare(&defs);
        let DefKind::Fundef { rec, .. } = &defs[1].0.kind else {
            panic!("expected Fundef, got {:?}", defs[1].0.kind)
        };
        let sail_ast::RecOpt::Measure { pat, exp } = rec else {
            panic!("expected RecOpt::Measure, got {rec:?}")
        };
        let args = lookup_function_args(&program, "f", 0, Span::DUMMY).unwrap();
        let expected = expected_pat_type(&args, pat.span);
        assert_eq!(expected, args[0]);
        assert_eq!(body_builtin(), "int");
        let _ = exp;
    }
}
