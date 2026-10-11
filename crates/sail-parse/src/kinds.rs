//! Definition-local kind inference before core AST conversion.
//!
//! Explicit and inferred binder kinds use the same constraints. This is not
//! full Sail typechecking: unknown user constructors require their declaration
//! environment, and expression typing remains a later compiler stage.

use std::collections::BTreeMap;

use crate::infix::{fold_chain_borrowed, FixityEnv, Folded, Side};
use sail_grammar::ast::{self as g, Kind};

pub(crate) type Bindings = BTreeMap<String, Kind>;

/// Surface argument sorts, including legacy Order parameters. Conversion
/// erases Order arguments only after checking their position and shape.
pub(crate) fn builtin_signature(name: &str, arity: usize) -> Option<(Vec<Kind>, Kind)> {
    use Kind::{Bool, Int, Order, Type};
    let (args, result) = match name {
        "int" | "atom" | "bits" | "implicit" | "itself" => (vec![Int], Type),
        "bitvector" if arity == 2 => (vec![Int, Order], Type),
        "bitvector" => (vec![Int], Type),
        "bool" | "atom_bool" => (vec![Bool], Type),
        "range" => (vec![Int, Int], Type),
        "list" | "option" | "register" => (vec![Type], Type),
        "result" => (vec![Type, Type], Type),
        "vector" if arity == 3 => (vec![Int, Order, Type], Type),
        "vector" => (vec![Int, Type], Type),
        "not" => (vec![Bool], Bool),
        "abs" => (vec![Int], Int),
        "div" | "mod" => (vec![Int, Int], Int),
        "ite" => (vec![Bool, Int, Int], Int),
        _ => return None,
    };
    Some((args, result))
}

fn merge(a: &Kind, b: &Kind) -> Result<Kind, String> {
    match (a, b) {
        (Kind::Int, Kind::Nat) | (Kind::Nat, Kind::Int) => Ok(Kind::Nat),
        _ if a == b => Ok(a.clone()),
        _ => Err(format!("conflicting kinds: {a:?} and {b:?}")),
    }
}

struct Inference<'a> {
    variables: BTreeMap<String, Option<Kind>>,
    outer: &'a Bindings,
    fixities: &'a FixityEnv,
}

impl Inference<'_> {
    fn variable(&mut self, name: &str, expected: Option<&Kind>) -> Result<Option<Kind>, String> {
        if let Some(value) = self.variables.get_mut(name) {
            if let Some(expected) = expected {
                *value = Some(match value.as_ref() {
                    Some(kind) => merge(kind, expected).map_err(|e| format!("{name}: {e}"))?,
                    None => expected.clone(),
                });
            }
            return Ok(value.clone());
        }
        match (self.outer.get(name), expected) {
            (Some(kind), Some(expected)) => Ok(Some(merge(kind, expected)?)),
            (Some(kind), None) => Ok(Some(kind.clone())),
            // Free variables can belong to a surrounding declaration. Their
            // existence is validated by whole-program elaboration, not here.
            (None, _) => Ok(expected.cloned()),
        }
    }

    fn result(actual: Option<Kind>, expected: Option<&Kind>) -> Result<Option<Kind>, String> {
        match (actual, expected) {
            (Some(actual), Some(expected)) => Ok(Some(merge(&actual, expected)?)),
            (actual, _) => Ok(actual.or_else(|| expected.cloned())),
        }
    }

    fn typ(&mut self, typ: &g::Typ, expected: Option<&Kind>) -> Result<Option<Kind>, String> {
        use Kind::{Bool, Int, Order, Type};
        let actual = match typ {
            g::Typ::Var(name) => return self.variable(name, expected),
            g::Typ::Id(name) => match name.as_str() {
                "true" | "false" => Some(Bool),
                "int" | "nat" | "bool" | "unit" | "bit" | "string" | "real" => Some(Type),
                _ => None,
            },
            g::Typ::Lit(g::Lit::Num(_) | g::Lit::Zero | g::Lit::One) => Some(Int),
            g::Typ::Lit(g::Lit::True | g::Lit::False) => Some(Bool),
            g::Typ::Lit(_) => return Err("invalid literal in a type expression".into()),
            g::Typ::Dec | g::Typ::Inc => Some(Order),
            g::Typ::Wild => None,
            g::Typ::NSet(_) => Some(Type),
            g::Typ::Tuple(items) => {
                for item in items {
                    self.typ(item, Some(&Type))?;
                }
                Some(Type)
            }
            g::Typ::Fn(lhs, rhs) => {
                self.typ(lhs, Some(&Type))?;
                self.typ(rhs, Some(&Type))?;
                Some(Type)
            }
            g::Typ::App(name, args) => {
                if let Some((kinds, result)) = builtin_signature(name, args.len()) {
                    if args.len() != kinds.len() {
                        return Err(format!("incorrect type argument count for {name}"));
                    }
                    for (arg, kind) in args.iter().zip(&kinds) {
                        self.typ(arg, Some(kind))?;
                    }
                    Some(result)
                } else {
                    for arg in args {
                        self.typ(arg, None)?;
                    }
                    None
                }
            }
            g::Typ::If(condition, yes, no) => {
                self.typ(condition, Some(&Bool))?;
                let yes_kind = self.typ(yes, expected)?;
                let no_kind = self.typ(no, expected.or(yes_kind.as_ref()))?;
                // Propagate a kind discovered on the right back to a variable
                // on the left. Later roots can also provide information.
                let actual = Self::result(yes_kind, no_kind.as_ref())?;
                self.typ(yes, actual.as_ref())?;
                actual
            }
            g::Typ::Infix(chain) => {
                let folded = fold_chain_borrowed(chain, Side::Typ, self.fixities)?;
                return self.folded(&folded, expected);
            }
            g::Typ::Exist(binders, constraint, body) => {
                let mut saved = Vec::new();
                for binder in binders {
                    for name in &binder.kids {
                        saved.push((
                            name.clone(),
                            self.variables.insert(name.clone(), binder.kind.clone()),
                        ));
                    }
                }
                let result: Result<Option<Kind>, String> = (|| {
                    self.typ(constraint, Some(&Bool))?;
                    self.typ(body, Some(&Type))?;
                    Ok(Some(Type))
                })();
                for (name, previous) in saved.into_iter().rev() {
                    match previous {
                        Some(previous) => {
                            self.variables.insert(name, previous);
                        }
                        None => {
                            self.variables.remove(&name);
                        }
                    }
                }
                result?
            }
        };
        Self::result(actual, expected)
    }

    fn folded(
        &mut self,
        tree: &Folded<&g::Typ>,
        expected: Option<&Kind>,
    ) -> Result<Option<Kind>, String> {
        use Kind::{Bool, Int};
        let actual = match tree {
            Folded::Leaf(typ) => return self.typ(typ, expected),
            Folded::Prefix { op, arg } if op == "pow2" || op == "negate" => {
                self.folded(arg, Some(&Int))?;
                Some(Int)
            }
            Folded::Prefix { op, .. } => return Err(format!("invalid type prefix {op}")),
            Folded::Infix { op, lhs, rhs } => match op.as_str() {
                "+" | "-" | "*" => {
                    self.folded(lhs, Some(&Int))?;
                    self.folded(rhs, Some(&Int))?;
                    Some(Int)
                }
                "<" | "<=" | ">" | ">=" | "=" | "==" | "!=" => {
                    self.folded(lhs, Some(&Int))?;
                    self.folded(rhs, Some(&Int))?;
                    Some(Bool)
                }
                "&" | "|" => {
                    self.folded(lhs, Some(&Bool))?;
                    self.folded(rhs, Some(&Bool))?;
                    Some(Bool)
                }
                "in" => {
                    self.folded(lhs, Some(&Int))?;
                    if !matches!(rhs.as_ref(), Folded::Leaf(g::Typ::NSet(_))) {
                        return Err("'in' requires a finite numeric set".into());
                    }
                    Some(Bool)
                }
                _ => {
                    self.folded(lhs, None)?;
                    self.folded(rhs, None)?;
                    None
                }
            },
        };
        Self::result(actual, expected)
    }
}

pub(crate) fn infer(
    quant: &g::TypQuant,
    roots: &[(&g::Typ, Kind)],
    outer: &Bindings,
    fixities: &FixityEnv,
) -> Result<Bindings, String> {
    let mut variables = BTreeMap::new();
    for binder in &quant.kopts {
        for name in &binder.kids {
            if variables
                .insert(name.clone(), binder.kind.clone())
                .is_some()
            {
                return Err(format!("duplicate type variable {name}"));
            }
        }
    }
    let mut inference = Inference {
        variables,
        outer,
        fixities,
    };
    // Monotone propagation: a variable acquires a kind or refines Int to Nat.
    // Revisit roots until stable, independent of their source traversal order.
    loop {
        let before = inference.variables.clone();
        if let Some(constraint) = &quant.constraint {
            inference.typ(constraint, Some(&Kind::Bool))?;
        }
        for (root, kind) in roots {
            inference.typ(root, Some(kind))?;
        }
        if before == inference.variables {
            break;
        }
    }
    inference.variables.into_iter().map(|(name, kind)| {
        kind.map(|kind| (name.clone(), kind)).ok_or_else(|| format!("cannot infer kind of {name}; its constructor declarations or kind annotation are required"))
    }).collect()
}

pub(crate) fn result_kind(
    quant: &g::TypQuant,
    body: &g::Typ,
    outer: &Bindings,
    fixities: &FixityEnv,
) -> Result<Kind, String> {
    let variables = quant
        .kopts
        .iter()
        .flat_map(|binder| {
            binder
                .kids
                .iter()
                .map(|name| (name.clone(), binder.kind.clone()))
        })
        .collect();
    let mut inference = Inference {
        variables,
        outer,
        fixities,
    };
    // An unknown nominal constructor can still denote a Type in a fragment;
    // resolving its declaration belongs to whole-program kind elaboration.
    Ok(inference.typ(body, None)?.unwrap_or(Kind::Type))
}
