//! `instantiate_outcomes` over the string-statement shard AST.
//!
//! Source: `sail/src/lib/rewrites.ml` (`instantiate_outcomes`, first entry
//! of the Lean `lean_rewrites` order). Sail `outcome` blocks declare an
//! abstract, parameterised result together with outcome-local `val`s; each
//! `instantiation` fixes the parameters for one concrete instance
//! (see `Program::outcomes` / `Program::instantiations` in
//! `sail-ty/src/program.rs`: headers are retained, bodies stay in the
//! driver store).
//!
//! Marker convention (everything else passes through untouched):
//!
//! - Template: `Def { kind: Other, body: ["outcome <O>", <content>...] }`.
//!   Content lines are `Stmt::Expr("param <k> = <default>")` parameter
//!   declarations and `Stmt::Expr("body <text>")` code lines; any other
//!   statement is template code too and undergoes word substitution.
//! - Instantiation: `Def { kind: Other, body:
//!   ["instantiation <O> as <I> [with <k1>=<v1>, ...]"] }`.
//! - Use site: the substring `outcome_use(<O>)` inside expression text.
//!
//! For every same-shard instantiation of a known outcome the pass appends
//! `Def { kind: Other, name: "<O>_<I>", body: ["outcome_inst <O> as <I>",
//! <substituted content>...] }`, replacing whole-word parameter occurrences
//! with their values (binders — `Assign` lhs, `TupleAssign` targets,
//! `LetTuple` bindings, `Undefined` targets — are never renamed). When an
//! outcome has exactly one same-shard instantiation, `outcome_use(<O>)`
//! markers in user code are rewritten to `outcome_use(<O>_<I>)`.
//!
//! Templates and instantiation declarations are kept; already-generated
//! `<O>_<I>` defs are not regenerated (`has_def`). Unknown outcomes,
//! malformed lines, and multi-instance use sites are left verbatim.
//! Shard-local, deterministic (instantiation order), and idempotent.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Def, DefKind, Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct InstantiateOutcomes;

impl Pass for InstantiateOutcomes {
    fn name(&self) -> &'static str {
        "instantiate_outcomes"
    }

    fn run(&self, shard: Shard) -> Shard {
        run(shard)
    }
}

fn is_ident(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn parse_outcome_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("outcome ")?.trim();
    if rest.contains(char::is_whitespace) || !is_ident(rest) {
        return None;
    }
    Some(rest.to_string())
}

fn parse_param(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("param ")?;
    let idx = rest.find('=')?;
    let (key, value) = rest.split_at(idx);
    let (key, value) = (key.trim(), value["=".len()..].trim());
    if !is_ident(key) || value.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

#[derive(Clone, Debug)]
struct Instantiation {
    outcome: String,
    instance: String,
    substs: BTreeMap<String, String>,
}

fn parse_instantiation(line: &str) -> Option<Instantiation> {
    let rest = line.strip_prefix("instantiation ")?.trim();
    let (head, substs_part) = match rest.find(" with ") {
        Some(idx) => {
            let (head, tail) = rest.split_at(idx);
            (head, Some(tail[" with ".len()..].trim()))
        }
        None => (rest, None),
    };
    let idx = head.find(" as ")?;
    let (outcome, instance) = head.split_at(idx);
    let (outcome, instance) = (
        outcome.trim().to_string(),
        instance[" as ".len()..].trim().to_string(),
    );
    if !is_ident(&outcome) || !is_ident(&instance) {
        return None;
    }
    let mut substs = BTreeMap::new();
    if let Some(part) = substs_part {
        if part.is_empty() {
            return None;
        }
        for entry in part.split(',') {
            let idx = entry.find('=')?;
            let (key, value) = entry.split_at(idx);
            let (key, value) = (key.trim(), value["=".len()..].trim());
            if !is_ident(key) || value.is_empty() {
                return None;
            }
            substs.insert(key.to_string(), value.to_string());
        }
    }
    Some(Instantiation {
        outcome,
        instance,
        substs,
    })
}

fn is_template_def(def: &Def) -> Option<String> {
    if def.kind != DefKind::Other {
        return None;
    }
    let [Stmt::Expr(header), ..] = def.body.as_slice() else {
        return None;
    };
    parse_outcome_header(header.trim())
}

fn is_instantiation_def(def: &Def) -> Option<Instantiation> {
    if def.kind != DefKind::Other {
        return None;
    }
    let [Stmt::Expr(line)] = def.body.as_slice() else {
        return None;
    };
    parse_instantiation(line.trim())
}

/// Replace whole-word occurrences of `subst` keys in expression text.
fn substitute_text(text: &str, substs: &BTreeMap<String, String>) -> String {
    if substs.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        match substs.get(token.as_str()) {
            Some(value) => out.push_str(value),
            None => out.push_str(token),
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

fn substitute_stmt(stmt: &Stmt, substs: &BTreeMap<String, String>) -> Stmt {
    match stmt {
        Stmt::Expr(line) => {
            if let Some((key, _)) = parse_param(line.trim()) {
                if let Some(value) = substs.get(&key) {
                    return Stmt::Expr(format!("param {key} = {value}"));
                }
                return stmt.clone();
            }
            if let Some(rest) = line.strip_prefix("body ") {
                return Stmt::Expr(format!("body {}", substitute_text(rest, substs)));
            }
            Stmt::Expr(substitute_text(line, substs))
        }
        Stmt::Assign { lhs, rhs } => Stmt::Assign {
            lhs: lhs.clone(),
            rhs: substitute_text(rhs, substs),
        },
        Stmt::TupleAssign { targets, rhs } => Stmt::TupleAssign {
            targets: targets.clone(),
            rhs: substitute_text(rhs, substs),
        },
        Stmt::LetTuple {
            bindings,
            rhs,
            body,
        } => Stmt::LetTuple {
            bindings: bindings.clone(),
            rhs: substitute_text(rhs, substs),
            body: body.iter().map(|s| substitute_stmt(s, substs)).collect(),
        },
        Stmt::Undefined { target, ty } => Stmt::Undefined {
            target: target.clone(),
            ty: substitute_text(ty, substs),
        },
        Stmt::Return(text) => Stmt::Return(substitute_text(text, substs)),
    }
}

fn rewrite_uses_in_stmt(stmt: &mut Stmt, outcome: &str, instance: &str) {
    let from = format!("outcome_use({outcome})");
    let to = format!("outcome_use({instance})");
    match stmt {
        Stmt::Expr(line) | Stmt::Return(line) => {
            *line = line.replace(&from, &to);
        }
        Stmt::Assign { rhs, .. } => {
            *rhs = rhs.replace(&from, &to);
        }
        Stmt::TupleAssign { rhs, .. } => {
            *rhs = rhs.replace(&from, &to);
        }
        Stmt::LetTuple { rhs, body, .. } => {
            *rhs = rhs.replace(&from, &to);
            for stmt in body {
                rewrite_uses_in_stmt(stmt, outcome, instance);
            }
        }
        Stmt::Undefined { ty, .. } => {
            *ty = ty.replace(&from, &to);
        }
    }
}

fn run(mut shard: Shard) -> Shard {
    let mut templates: BTreeMap<String, Vec<Stmt>> = BTreeMap::new();
    for def in &shard.defs {
        if let Some(name) = is_template_def(def) {
            templates
                .entry(name)
                .or_insert_with(|| def.body[1..].to_vec());
        }
    }
    let mut instantiations: Vec<Instantiation> = Vec::new();
    for def in &shard.defs {
        if let Some(inst) = is_instantiation_def(def) {
            instantiations.push(inst);
        }
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for inst in &instantiations {
        *counts.entry(inst.outcome.clone()).or_insert(0) += 1;
    }
    let single: BTreeMap<String, String> = instantiations
        .iter()
        .filter(|inst| counts.get(&inst.outcome).copied().unwrap_or(0) == 1)
        .map(|inst| {
            (
                inst.outcome.clone(),
                format!("{}_{}", inst.outcome, inst.instance),
            )
        })
        .collect();

    let mut existing: BTreeSet<String> = shard.defs.iter().map(|d| d.name.clone()).collect();
    let mut generated: Vec<Def> = Vec::new();
    for inst in &instantiations {
        let Some(template) = templates.get(&inst.outcome) else {
            continue;
        };
        let name = format!("{}_{}", inst.outcome, inst.instance);
        if !existing.insert(name.clone()) {
            continue;
        }
        let mut body = vec![Stmt::Expr(format!(
            "outcome_inst {} as {}",
            inst.outcome, inst.instance
        ))];
        body.extend(
            template
                .iter()
                .map(|stmt| substitute_stmt(stmt, &inst.substs)),
        );
        generated.push(Def::new(name, DefKind::Other, body));
    }

    // Rewrite single-instance use sites in user code only (never inside
    // templates, instantiation declarations, or generated instances).
    for def in &mut shard.defs {
        if is_template_def(def).is_some() || is_instantiation_def(def).is_some() {
            continue;
        }
        if def.body.first().is_some_and(|stmt| {
            matches!(stmt, Stmt::Expr(line) if line.trim_start().starts_with("outcome_inst "))
        }) {
            continue;
        }
        for (outcome, instance) in &single {
            for stmt in &mut def.body {
                rewrite_uses_in_stmt(stmt, outcome, instance);
            }
        }
    }

    shard.defs.extend(generated);
    shard
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> Def {
        Def::new(
            "cache_op",
            DefKind::Other,
            vec![
                Stmt::Expr("outcome cache_op".into()),
                Stmt::Expr("param ADDR = 64".into()),
                Stmt::Expr("body write ADDR with outcome_use(cache_op)".into()),
                Stmt::Assign {
                    lhs: "ADDR".into(),
                    rhs: "ADDR + 1".into(),
                },
            ],
        )
    }

    fn instantiation() -> Def {
        Def::new(
            "cache_op_inst",
            DefKind::Other,
            vec![Stmt::Expr(
                "instantiation cache_op as clean with ADDR=32".into(),
            )],
        )
    }

    fn run_defs(defs: Vec<Def>) -> Vec<Def> {
        InstantiateOutcomes.run(Shard::new(0, defs)).defs
    }

    #[test]
    fn single_instantiation_emits_specialised_def_with_substitution() {
        let out = run_defs(vec![template(), instantiation()]);
        let inst = out
            .iter()
            .find(|d| d.name == "cache_op_clean")
            .expect("instance");
        assert_eq!(inst.kind, DefKind::Other);
        assert_eq!(
            inst.body[0],
            Stmt::Expr("outcome_inst cache_op as clean".into())
        );
        assert_eq!(inst.body[1], Stmt::Expr("param ADDR = 32".into()));
        assert_eq!(
            inst.body[2],
            Stmt::Expr("body write 32 with outcome_use(cache_op)".into())
        );
        // Binders are never renamed; only expression positions substitute.
        assert_eq!(
            inst.body[3],
            Stmt::Assign {
                lhs: "ADDR".into(),
                rhs: "32 + 1".into()
            }
        );
        // Templates and declarations are kept.
        assert!(out.iter().any(|d| d.name == "cache_op"));
        assert!(out.iter().any(|d| d.name == "cache_op_inst"));
    }

    #[test]
    fn single_instance_use_sites_are_rewritten() {
        let user = Def::new(
            "f",
            DefKind::Function,
            vec![
                Stmt::Expr("outcome_use(cache_op)".into()),
                Stmt::Assign {
                    lhs: "x".into(),
                    rhs: "outcome_use(cache_op)".into(),
                },
                Stmt::Return("outcome_use(cache_op)".into()),
            ],
        );
        let out = run_defs(vec![template(), instantiation(), user]);
        let f = out.iter().find(|d| d.name == "f").expect("user code");
        assert_eq!(f.body[0], Stmt::Expr("outcome_use(cache_op_clean)".into()));
        assert_eq!(
            f.body[1],
            Stmt::Assign {
                lhs: "x".into(),
                rhs: "outcome_use(cache_op_clean)".into()
            }
        );
        assert_eq!(
            f.body[2],
            Stmt::Return("outcome_use(cache_op_clean)".into())
        );
    }

    #[test]
    fn multiple_instances_emit_all_but_leave_uses_ambiguous() {
        let second = Def::new(
            "cache_op_inst2",
            DefKind::Other,
            vec![Stmt::Expr(
                "instantiation cache_op as dirty with ADDR=48".into(),
            )],
        );
        let user = Def::new(
            "f",
            DefKind::Function,
            vec![Stmt::Expr("outcome_use(cache_op)".into())],
        );
        let out = run_defs(vec![template(), instantiation(), second, user]);
        assert!(out.iter().any(|d| d.name == "cache_op_clean"));
        assert!(out.iter().any(|d| d.name == "cache_op_dirty"));
        let f = out.iter().find(|d| d.name == "f").expect("user code");
        assert_eq!(f.body, vec![Stmt::Expr("outcome_use(cache_op)".into())]);
    }

    #[test]
    fn unknown_outcome_malformed_and_existing_pass_through() {
        let unknown = Def::new(
            "x_inst",
            DefKind::Other,
            vec![Stmt::Expr("instantiation nope as i with A=1".into())],
        );
        let malformed = Def::new(
            "bad",
            DefKind::Other,
            vec![Stmt::Expr("instantiation cache_op as".into())],
        );
        let pre = Def::new(
            "cache_op_clean",
            DefKind::Other,
            vec![Stmt::Expr("kept".into())],
        );
        let out = run_defs(vec![
            template(),
            instantiation(),
            unknown,
            malformed,
            pre.clone(),
        ]);
        // `has_def`: the pre-existing instance wins, no duplicate emitted.
        let found: Vec<_> = out.iter().filter(|d| d.name == "cache_op_clean").collect();
        assert_eq!(found, vec![&pre]);
        assert!(out.iter().any(|d| d.name == "x_inst"));
        assert!(out.iter().any(|d| d.name == "bad"));
    }

    #[test]
    fn pass_is_idempotent() {
        let user = Def::new(
            "f",
            DefKind::Function,
            vec![Stmt::Expr("outcome_use(cache_op)".into())],
        );
        let once = run_defs(vec![template(), instantiation(), user]);
        let twice = InstantiateOutcomes.run(Shard::new(0, once.clone())).defs;
        assert_eq!(twice, once);
    }
}
