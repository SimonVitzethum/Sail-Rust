//! Fold bit-literal vectors into single bitvector literals.
//!
//! Mirrors `rewrite_bit_lists_to_lits` in `sail/src/lib/rewrites.ml`:
//! `E_vector [0b1; 0b0; ...]` whose elements are *all* single-bit literals
//! becomes one `E_lit`. The literal is `L_hex` when the bit count is a
//! non-empty multiple of four (OCaml `BitList.to_hex_digits`), otherwise
//! `L_bin`. Bit order follows list order (most significant bit first) and
//! the total length is preserved. Doubly-annotated `E_typ (t, E_typ (_, e))`
//! collapses to `E_typ (t, e)`, as in OCaml.
//!
//! `check_exp` / `is_bitvector_typ` gap: OCaml only folds when the vector's
//! *checked* type is a bitvector (`is_bitvector_typ typ`). The untyped
//! [`sail_ast`] IR carries no type environment, so [`BitListsToLits::run_ast`]
//! uses the adjacent syntactic gate — every element must be a single-bit
//! `L_bin` literal (`"0"` / `"1"`, exactly as the parser produces for `0b0`
//! / `0b1`; multi-bit `L_bin` texts are rejected like OCaml's
//! `L_bin [Non_empty (b, [])]` shape test). A `E_vector` of single-bit
//! literals with a non-bitvector type (e.g. `vector(2, bits(1))`) would be
//! skipped by OCaml but is folded here; the future `TypedPass` wiring must
//! re-add the `is_bitvector_typ` gate once type information is available.
//!
//! String-level [`crate::pass::Pass`] is the identity transform: the
//! statement IR has no type information, so the gate above cannot be
//! evaluated (the same conservative choice as `SimpleAssignments`, which
//! preserves complex lvalues without a type environment).

#![forbid(unsafe_code)]

use sail_ast::Interner;

use crate::ast::Shard;
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct BitListsToLits;

impl Pass for BitListsToLits {
    fn name(&self) -> &'static str {
        "bit_lists_to_lits"
    }

    fn run(&self, shard: Shard) -> Shard {
        shard
    }
}

impl crate::pass::TypedPass for BitListsToLits {
    fn name(&self) -> &'static str { "bit_lists_to_lits" }
    fn run(&self, ast: &mut sail_ast::Ast, interner: &mut dyn sail_ast::Interner) {
        Self::run_ast(self, ast, interner);
    }
}

impl BitListsToLits {
    /// Fold every bit-literal vector in `ast`, bottom-up.
    ///
    /// Returns the number of rewritten nodes.
    pub fn run_ast(
        &self,
        ast: &mut sail_ast::Ast,
        interner: &mut dyn sail_ast::Interner,
    ) -> usize {
        let mut count = 0;
        for def in &mut ast.defs {
            walk_def(def, &mut |exp| {
                if rewrite_exp(exp, interner) {
                    count += 1;
                }
            });
        }
        count
    }
}

/// Try to rewrite one expression node. Returns whether it changed.
fn rewrite_exp(exp: &mut sail_ast::Exp, interner: &mut dyn sail_ast::Interner) -> bool {
    match &mut exp.kind {
        sail_ast::ExpKind::Vector(es) => {
            let mut bits = Vec::with_capacity(es.len());
            for e in es.iter() {
                let Some(bit) = bit_of_exp(e, interner) else {
                    return false;
                };
                bits.push(bit);
            }
            if bits.is_empty() {
                // OCaml `lit_of_bits []` cannot build a non-empty literal
                // (`non_empty_singleton []`); the enclosing `try ... with _`
                // leaves empty vectors alone. Match that observable behavior.
                return false;
            }
            exp.kind = sail_ast::ExpKind::Lit(lit_for_bits(&bits, interner));
            true
        }
        sail_ast::ExpKind::Typ { exp: inner, .. } => {
            // `E_typ (typ', E_typ (_, e'))` -> `E_typ (typ', e')`.
            // Restore the node untouched when the inner expression is not
            // itself annotated.
            let old = std::mem::replace(&mut inner.kind, sail_ast::ExpKind::Undef);
            match old {
                sail_ast::ExpKind::Typ { exp: innermost, .. } => {
                    inner.kind = innermost.kind;
                    true
                }
                other => {
                    inner.kind = other;
                    false
                }
            }
        }
        _ => false,
    }
}

/// OCaml `bit_of_exp`: `Some` for single-bit literals only.
fn bit_of_exp(exp: &sail_ast::Exp, interner: &dyn sail_ast::Interner) -> Option<bool> {
    let sail_ast::ExpKind::Lit(lit) = &exp.kind else {
        return None;
    };
    bit_of_lit(lit, interner)
}

/// OCaml `bit_of_lit`: `L_bin` holding exactly one binary digit.
fn bit_of_lit(lit: &sail_ast::Lit, interner: &dyn sail_ast::Interner) -> Option<bool> {
    let sail_ast::LitKind::Bin(symbol) = lit.kind else {
        return None;
    };
    match interner.resolve(symbol) {
        Some("0") => Some(false),
        Some("1") => Some(true),
        _ => None,
    }
}

/// OCaml `lit_of_bits`: hex when the bits form whole nibbles, else binary.
///
/// The bit slice must be non-empty (see [`rewrite_exp`]).
fn lit_for_bits(bits: &[bool], interner: &mut dyn sail_ast::Interner) -> sail_ast::Lit {
    if bits.len().is_multiple_of(4) {
        let mut text = String::with_capacity(bits.len() / 4);
        for nibble in bits.chunks_exact(4) {
            let digit = (nibble[0] as u8) * 8 + (nibble[1] as u8) * 4 + (nibble[2] as u8) * 2
                + (nibble[3] as u8);
            text.push(char::from_digit(u32::from(digit), 16).expect("nibble is hex"));
        }
        let text = text.to_ascii_uppercase();
        sail_ast::Node::dummy(sail_ast::LitKind::Hex(interner.intern(&text)))
    } else {
        let text: String = bits.iter().map(|b| if *b { '1' } else { '0' }).collect();
        sail_ast::Node::dummy(sail_ast::LitKind::Bin(interner.intern(&text)))
    }
}

// ---------------------------------------------------------------------------
// Bottom-up expression walk (OCaml `fold_exp` order: children first).
// ---------------------------------------------------------------------------

fn walk_def(def: &mut sail_ast::Def, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut def.kind {
        sail_ast::DefKind::Fundef { funcls, .. } => {
            for funcl in funcls {
                walk_funcl(funcl, f);
            }
        }
        sail_ast::DefKind::Mapdef { clauses, .. } => {
            for clause in clauses {
                walk_mapcl(clause, f);
            }
        }
        sail_ast::DefKind::Impl(funcl) => walk_funcl(funcl, f),
        sail_ast::DefKind::Let { exp, .. } => walk_exp(exp, f),
        sail_ast::DefKind::Outcome { defs, .. } => {
            for def in defs {
                walk_def(def, f);
            }
        }
        sail_ast::DefKind::Measure { exp, .. } => walk_exp(exp, f),
        // OCaml `rewrite_def` leaves `DEF_loop_measures` alone ("haven't
        // been typechecked").
        sail_ast::DefKind::LoopMeasures { .. } => {}
        sail_ast::DefKind::Register(spec) => {
            if let Some(init) = &mut spec.init {
                walk_exp(init, f);
            }
        }
        sail_ast::DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &mut set.funcls {
                    walk_funcl(funcl, f);
                }
            }
        }
        sail_ast::DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => walk_funcl(funcl, f),
            sail_ast::ScatteredDef::Mapcl { clause, .. } => walk_mapcl(clause, f),
            _ => {}
        },
        _ => {}
    }
}

fn walk_funcl(funcl: &mut sail_ast::Funcl, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    walk_pexp(&mut funcl.arm, f);
}

fn walk_mapcl(clause: &mut sail_ast::Mapcl, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut clause.kind {
        sail_ast::MapclKind::Bidir { forwards, backwards } => {
            walk_mpexp(forwards, f);
            walk_mpexp(backwards, f);
        }
        sail_ast::MapclKind::Forwards(pexp) | sail_ast::MapclKind::Backwards(pexp) => {
            walk_pexp(pexp, f);
        }
    }
}

fn walk_mpexp(mpexp: &mut sail_ast::MPExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    if let sail_ast::MPExpKind::When { guard, .. } = &mut mpexp.kind {
        walk_exp(guard, f);
    }
}

fn walk_pexp(pexp: &mut sail_ast::PExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut pexp.kind {
        sail_ast::PExpKind::PatExp { body, .. } => walk_exp(body, f),
        sail_ast::PExpKind::PatWhen { guard, body, .. } => {
            walk_exp(guard, f);
            walk_exp(body, f);
        }
    }
}

fn walk_lexp(lexp: &mut sail_ast::LExp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut lexp.kind {
        sail_ast::LExpKind::Deref(exp) => walk_exp(exp, f),
        sail_ast::LExpKind::Vector { lexp, index } => {
            walk_lexp(lexp, f);
            walk_exp(index, f);
        }
        sail_ast::LExpKind::VectorRange { lexp, lo, hi } => {
            walk_lexp(lexp, f);
            walk_exp(lo, f);
            walk_exp(hi, f);
        }
        sail_ast::LExpKind::Field { lexp, .. } => walk_lexp(lexp, f),
        sail_ast::LExpKind::Tuple(parts) | sail_ast::LExpKind::VectorConcat(parts) => {
            for part in parts {
                walk_lexp(part, f);
            }
        }
        sail_ast::LExpKind::Id(_) | sail_ast::LExpKind::Typ { .. } => {}
    }
}

#[allow(clippy::too_many_lines)]
fn walk_exp(exp: &mut sail_ast::Exp, f: &mut impl FnMut(&mut sail_ast::Exp)) {
    match &mut exp.kind {
        sail_ast::ExpKind::Block(es)
        | sail_ast::ExpKind::Tuple(es)
        | sail_ast::ExpKind::Vector(es)
        | sail_ast::ExpKind::List(es) => {
            for e in es {
                walk_exp(e, f);
            }
        }
        sail_ast::ExpKind::Typ { exp: inner, .. } => walk_exp(inner, f),
        sail_ast::ExpKind::App { args, .. } => {
            for e in args {
                walk_exp(e, f);
            }
        }
        sail_ast::ExpKind::If { cond, then_branch, else_branch } => {
            walk_exp(cond, f);
            walk_exp(then_branch, f);
            walk_exp(else_branch, f);
        }
        sail_ast::ExpKind::Loop { measure, cond, body, .. } => {
            if let Some(measure) = measure {
                walk_exp(measure, f);
            }
            walk_exp(cond, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::For { from, to, step, body, .. } => {
            walk_exp(from, f);
            walk_exp(to, f);
            walk_exp(step, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::VectorAppend(first, second) | sail_ast::ExpKind::Cons(first, second) => {
            walk_exp(first, f);
            walk_exp(second, f);
        }
        sail_ast::ExpKind::Struct { fields, .. } => {
            for field in fields {
                walk_exp(&mut field.value, f);
            }
        }
        sail_ast::ExpKind::StructUpdate { record, fields } => {
            walk_exp(record, f);
            for field in fields {
                walk_exp(&mut field.value, f);
            }
        }
        sail_ast::ExpKind::Field { record, .. } => walk_exp(record, f),
        sail_ast::ExpKind::Match { scrutinee, arms } => {
            walk_exp(scrutinee, f);
            for arm in arms {
                walk_pexp(arm, f);
            }
        }
        sail_ast::ExpKind::Let { bind, body, .. } => {
            walk_exp(bind, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::Assign { lexp, value } => {
            walk_lexp(lexp, f);
            walk_exp(value, f);
        }
        sail_ast::ExpKind::Return(inner)
        | sail_ast::ExpKind::Exit(inner)
        | sail_ast::ExpKind::Throw(inner)
        | sail_ast::ExpKind::InternalReturn(inner) => walk_exp(inner, f),
        sail_ast::ExpKind::Try { exp: inner, arms } => {
            walk_exp(inner, f);
            for arm in arms {
                walk_pexp(arm, f);
            }
        }
        sail_ast::ExpKind::Assert { cond, msg } => {
            walk_exp(cond, f);
            walk_exp(msg, f);
        }
        sail_ast::ExpKind::Var { lexp, init, body } => {
            walk_lexp(lexp, f);
            walk_exp(init, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::InternalPlet { bind, body, .. } => {
            walk_exp(bind, f);
            walk_exp(body, f);
        }
        sail_ast::ExpKind::InternalAssume { exp: inner, .. } => walk_exp(inner, f),
        sail_ast::ExpKind::Id(_)
        | sail_ast::ExpKind::Lit(_)
        | sail_ast::ExpKind::Sizeof(_)
        | sail_ast::ExpKind::Config(_)
        | sail_ast::ExpKind::Ref(_)
        | sail_ast::ExpKind::Undef
        | sail_ast::ExpKind::InternalValue(_)
        | sail_ast::ExpKind::Constraint(_) => {}
    }
    f(exp);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bit_exp(it: &mut sail_ast::SimpleInterner, text: &str) -> sail_ast::Exp {
        sail_ast::Node::dummy(sail_ast::ExpKind::Lit(sail_ast::Node::dummy(
            sail_ast::LitKind::Bin(it.intern(text)),
        )))
    }

    fn vector_ast(es: Vec<sail_ast::Exp>) -> sail_ast::Ast {
        sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
            exp: sail_ast::Node::dummy(sail_ast::ExpKind::Vector(es)),
        })])
    }

    fn single_exp(ast: &sail_ast::Ast) -> &sail_ast::Exp {
        let sail_ast::DefKind::Let { exp, .. } = &ast.defs[0].kind else {
            panic!("def shape")
        };
        exp
    }

    #[test]
    fn four_bits_fold_to_hex() {
        let mut it = sail_ast::SimpleInterner::new();
        let mut ast = vector_ast(vec![
            bit_exp(&mut it, "1"),
            bit_exp(&mut it, "0"),
            bit_exp(&mut it, "1"),
            bit_exp(&mut it, "0"),
        ]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 1);
        let sail_ast::ExpKind::Lit(lit) = &single_exp(&ast).kind else {
            panic!("expected literal, got {:?}", single_exp(&ast).kind)
        };
        let sail_ast::LitKind::Hex(symbol) = lit.kind else {
            panic!("expected hex, got {:?}", lit.kind)
        };
        assert_eq!(it.resolve(symbol), Some("A"));
        // Idempotent: a literal is not a vector.
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 0);
    }

    #[test]
    fn three_bits_fold_to_bin_preserving_order() {
        let mut it = sail_ast::SimpleInterner::new();
        let mut ast = vector_ast(vec![
            bit_exp(&mut it, "1"),
            bit_exp(&mut it, "0"),
            bit_exp(&mut it, "1"),
        ]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 1);
        let sail_ast::ExpKind::Lit(lit) = &single_exp(&ast).kind else {
            panic!("expected literal")
        };
        let sail_ast::LitKind::Bin(symbol) = lit.kind else {
            panic!("expected bin, got {:?}", lit.kind)
        };
        assert_eq!(it.resolve(symbol), Some("101"));
    }

    #[test]
    fn multi_bit_elements_empty_and_mixed_vectors_survive() {
        let mut it = sail_ast::SimpleInterner::new();
        // Multi-bit `L_bin` is not a single bit (OCaml shape test).
        let mut ast = vector_ast(vec![bit_exp(&mut it, "10")]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 0);
        // Empty vectors survive (OCaml `try ... with _`).
        let mut ast = vector_ast(vec![]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 0);
        // One non-bit element vetoes the whole vector.
        let mut ast = vector_ast(vec![
            bit_exp(&mut it, "1"),
            sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(it.intern("x")))),
        ]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 0);
        assert!(matches!(single_exp(&ast).kind, sail_ast::ExpKind::Vector(_)));
    }

    #[test]
    fn nested_vectors_fold_inside_out_without_cascading() {
        let mut it = sail_ast::SimpleInterner::new();
        // Inner `[1; 0]` (length 2, not a nibble) folds to `0b10`; the outer
        // vector then holds a multi-bit literal and must survive.
        let mut ast = vector_ast(vec![
            sail_ast::Node::dummy(sail_ast::ExpKind::Vector(vec![
                bit_exp(&mut it, "1"),
                bit_exp(&mut it, "0"),
            ])),
            bit_exp(&mut it, "1"),
        ]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 1);
        let sail_ast::ExpKind::Vector(es) = &single_exp(&ast).kind else {
            panic!("outer vector must survive")
        };
        assert_eq!(es.len(), 2);
        assert!(matches!(es[0].kind, sail_ast::ExpKind::Lit(_)));
    }

    #[test]
    fn double_typ_annotation_collapses() {
        let mut it = sail_ast::SimpleInterner::new();
        let inner = sail_ast::Node::dummy(sail_ast::ExpKind::Id(sail_ast::Name::dummy(
            it.intern("x"),
        )));
        let mid = sail_ast::Node::dummy(sail_ast::ExpKind::Typ {
            typ: sail_ast::Node::dummy(sail_ast::TypKind::Unknown),
            exp: Box::new(inner),
        });
        let mut ast = sail_ast::Ast::new(vec![sail_ast::Node::dummy(sail_ast::DefKind::Let {
            pat: sail_ast::Node::dummy(sail_ast::PatKind::Wild),
            exp: sail_ast::Node::dummy(sail_ast::ExpKind::Typ {
                typ: sail_ast::Node::dummy(sail_ast::TypKind::Unknown),
                exp: Box::new(mid),
            }),
        })]);
        assert_eq!(BitListsToLits.run_ast(&mut ast, &mut it), 1);
        let sail_ast::ExpKind::Typ { exp, .. } = &single_exp(&ast).kind else {
            panic!("expected single annotation")
        };
        assert!(matches!(exp.kind, sail_ast::ExpKind::Id(_)));
    }

    #[test]
    fn string_level_is_identity() {
        use crate::ast::{Def, DefKind, Stmt};
        let shard = Shard::new(
            0,
            vec![Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Assign { lhs: "x".into(), rhs: "[0b1, 0b0]".into() }],
            )],
        );
        assert_eq!(BitListsToLits.run(shard.clone()), shard);
        assert_eq!(BitListsToLits.name(), "bit_lists_to_lits");
    }
}
