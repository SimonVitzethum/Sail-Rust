//! `mapping_patterns` over the string-statement shard AST.
//!
//! Source: `sail/src/lib/mappings.ml`. After `realize_mappings`, mapping
//! *applications* can still occur as patterns, e.g.
//!
//! ```sail
//! mapping hex_bits_2 : bits(2) <-> string = { hex_bits(2, s) <-> s }
//! match x { hex_bits_2(ta @ sew) => e, ... }
//! ```
//!
//! OCaml elaborates such an arm into a guarded nested match: bind the
//! scrutinee, guard on `hex_bits_2_backwards_matches`, then match on
//! `hex_bits_2_backwards(scrutinee)`.
//!
//! Marker convention (everything else passes through untouched): match arms
//! are `Stmt::Expr("match_arm <pat> [when <guard>] => <body>")`. A mapping
//! application inside `<pat>` is written `<name>/forwards(<inner>)` or
//! `<name>/backwards(<inner>)`, mirroring the `forwards`/`backwards`
//! pattern attributes (`mapping_direction`). The pass rewrites the arm to
//!
//! ```text
//! match_elab vars=[__mapping_0] guards=[<m>_<dir>_matches(__mapping_0)]
//!   calls=[<m>_<dir>(__mapping_0)] inner=<subpat> [when <guard>] => <body>
//! ```
//!
//! `is_mapping` is answered shard-locally: `<m>` counts as a mapping when
//! the same shard holds `<m>_forwards`, `<m>_backwards`,
//! `<m>_forwards_matches`, `<m>_backwards_matches`, or a `realize_mappings`
//! mapping source named `<m>`. Arms without known mappings, malformed arms
//! (missing `=>`, unbalanced parens), and already-elaborated `match_elab`
//! lines are preserved verbatim. Fresh names (`__mapping_<n>`) avoid every
//! identifier already present in the definition.
//!
//! Shard-local, deterministic (left-to-right extraction, stable numbering
//! per definition), and idempotent.

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashSet};

use crate::ast::{Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct MappingPatterns;

impl Pass for MappingPatterns {
    fn name(&self) -> &'static str {
        "mapping_patterns"
    }

    fn run(&self, shard: Shard) -> Shard {
        run(shard)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MappingUse {
    subst_id: String,
    mapping: String,
    direction: String,
    subpat: String,
}

const SUFFIXES: [&str; 4] = [
    "_forwards_matches",
    "_backwards_matches",
    "_forwards",
    "_backwards",
];

fn mapping_names(shard: &Shard) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for def in &shard.defs {
        for suffix in SUFFIXES {
            if let Some(base) = def.name.strip_suffix(suffix) {
                if !base.is_empty() {
                    names.insert(base.to_string());
                }
            }
        }
        if def.kind == crate::ast::DefKind::Other {
            if let [Stmt::Expr(header), ..] = def.body.as_slice() {
                let trimmed = header.trim();
                if let Some(rest) = trimmed.strip_prefix("mapping ") {
                    let head = rest.split(':').next().unwrap_or("").trim();
                    if head == def.name {
                        names.insert(def.name.clone());
                    }
                }
            }
        }
    }
    names
}

fn collect_identifiers(body: &[Stmt], used: &mut BTreeSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Assign { lhs, rhs } => {
                idents(lhs, used);
                idents(rhs, used);
            }
            Stmt::TupleAssign { targets, rhs } => {
                for target in targets {
                    idents(target, used);
                }
                idents(rhs, used);
            }
            Stmt::LetTuple {
                bindings,
                rhs,
                body,
            } => {
                for binding in bindings {
                    idents(binding, used);
                }
                idents(rhs, used);
                collect_identifiers(body, used);
            }
            Stmt::Undefined { target, ty } => {
                idents(target, used);
                idents(ty, used);
            }
            Stmt::Expr(src) | Stmt::Return(src) => idents(src, used),
        }
    }
}

fn idents(src: &str, used: &mut BTreeSet<String>) {
    for token in src.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if !token.is_empty() {
            used.insert(token.to_string());
        }
    }
}

struct Fresh {
    next: usize,
    used: BTreeSet<String>,
}

impl Fresh {
    fn fresh(&mut self) -> String {
        loop {
            let name = format!("__mapping_{}", self.next);
            self.next += 1;
            if self.used.insert(name.clone()) {
                return name;
            }
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Extract mapping applications from `pat`, replacing each known one with a
/// fresh variable. Returns `None` on unbalanced parens.
fn extract(
    pat: &str,
    is_mapping: &HashSet<String>,
    fresh: &mut Fresh,
) -> Option<(String, Vec<MappingUse>)> {
    let bytes = pat.as_bytes();
    let mut out = String::with_capacity(pat.len());
    let mut uses = Vec::new();
    let mut i = 0;
    while i < pat.len() {
        let c = bytes[i] as char;
        if is_ident_char(c) {
            let mut j = i;
            while j < pat.len() && is_ident_char(bytes[j] as char) {
                j += 1;
            }
            let ident = &pat[i..j];
            let mut dir: Option<&str> = None;
            let mut inner_start = 0;
            if pat[j..].starts_with("/forwards(") {
                dir = Some("forwards");
                inner_start = j + "/forwards(".len();
            } else if pat[j..].starts_with("/backwards(") {
                dir = Some("backwards");
                inner_start = j + "/backwards(".len();
            }
            if let Some(direction) = dir {
                let mut depth = 1usize;
                let mut k = inner_start;
                while k < pat.len() {
                    match bytes[k] as char {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                if depth != 0 || k >= pat.len() {
                    return None;
                }
                let inner = &pat[inner_start..k];
                // Recurse so nested mapping applications inside `inner`
                // get their own fresh variables first.
                let (inner_pat, mut inner_uses) = extract(inner, is_mapping, fresh)?;
                uses.append(&mut inner_uses);
                if is_mapping.contains(ident) {
                    let id = fresh.fresh();
                    uses.push(MappingUse {
                        subst_id: id.clone(),
                        mapping: ident.to_string(),
                        direction: direction.to_string(),
                        subpat: inner_pat,
                    });
                    out.push_str(&id);
                } else {
                    out.push_str(ident);
                    out.push('/');
                    out.push_str(direction);
                    out.push('(');
                    out.push_str(&inner_pat);
                    out.push(')');
                }
                i = k + 1;
                continue;
            }
            out.push_str(ident);
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Some((out, uses))
}

fn elaborate_arm(line: &str, is_mapping: &HashSet<String>, fresh: &mut Fresh) -> Option<Stmt> {
    let rest = line.strip_prefix("match_arm ")?;
    let idx = rest.find("=>")?;
    let (left, body) = rest.split_at(idx);
    let body = body["=>".len()..].trim();
    if body.is_empty() {
        return None;
    }
    let (pat, guard) = match left.find(" when ") {
        Some(g) => {
            let (pat, guard) = left.split_at(g);
            let (pat, guard) = (pat.trim(), guard[" when ".len()..].trim());
            if pat.is_empty() || guard.is_empty() {
                return None;
            }
            (pat.to_string(), Some(guard.to_string()))
        }
        None => {
            let pat = left.trim();
            if pat.is_empty() {
                return None;
            }
            (pat.to_string(), None)
        }
    };
    let (_rewritten, uses) = extract(&pat, is_mapping, fresh)?;
    if uses.is_empty() {
        return None;
    }
    let vars: Vec<String> = uses.iter().map(|u| u.subst_id.clone()).collect();
    let guards: Vec<String> = uses
        .iter()
        .map(|u| format!("{}_{}_matches({})", u.mapping, u.direction, u.subst_id))
        .collect();
    let calls: Vec<String> = uses
        .iter()
        .map(|u| format!("{}_{}({})", u.mapping, u.direction, u.subst_id))
        .collect();
    let inner = if uses.len() == 1 {
        uses[0].subpat.clone()
    } else {
        format!(
            "({})",
            uses.iter()
                .map(|u| u.subpat.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    // The rewritten pattern is implied by `vars` (each var replaced one
    // mapping application); keep the text for debuggability in the plan.
    let mut text = format!(
        "match_elab vars=[{}] guards=[{}] calls=[{}] inner={}",
        vars.join(", "),
        guards.join(" && "),
        calls.join(", "),
        inner
    );
    if let Some(guard) = guard {
        text.push_str(&format!(" when {guard}"));
    }
    text.push_str(&format!(" => {body}"));
    Some(Stmt::Expr(text))
}

fn rewrite_body(body: &mut Vec<Stmt>, is_mapping: &HashSet<String>, fresh: &mut Fresh) {
    for stmt in body {
        match stmt {
            Stmt::Expr(line) if line.trim_start().starts_with("match_arm ") => {
                let trimmed = line.trim().to_string();
                if let Some(elab) = elaborate_arm(&trimmed, is_mapping, fresh) {
                    *line = match elab {
                        Stmt::Expr(text) => text,
                        other => {
                            *stmt = other;
                            continue;
                        }
                    };
                }
            }
            Stmt::LetTuple { body, .. } => rewrite_body(body, is_mapping, fresh),
            _ => {}
        }
    }
}

fn run(mut shard: Shard) -> Shard {
    let names = mapping_names(&shard);
    let is_mapping: HashSet<String> = names.into_iter().collect();
    for def in &mut shard.defs {
        let mut used = BTreeSet::new();
        collect_identifiers(&def.body, &mut used);
        let mut fresh = Fresh { next: 0, used };
        rewrite_body(&mut def.body, &is_mapping, &mut fresh);
    }
    shard
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Def, DefKind};

    fn shard_with(defs: Vec<Def>) -> Shard {
        Shard::new(0, defs)
    }

    fn table(name: &str) -> Def {
        Def::new(format!("{name}_forwards"), DefKind::Function, vec![])
    }

    fn arm_body(defs: Vec<Def>) -> Vec<Stmt> {
        MappingPatterns
            .run(shard_with(defs))
            .defs
            .swap_remove(0)
            .body
    }

    #[test]
    fn mapping_pattern_arm_produces_guard_and_call() {
        let body = arm_body(vec![
            Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Expr(
                    "match_arm hex_bits_2/backwards(ta @ sew) => e".into(),
                )],
            ),
            table("hex_bits_2"),
        ]);
        assert_eq!(
            body,
            vec![Stmt::Expr(
                "match_elab vars=[__mapping_0] guards=[hex_bits_2_backwards_matches(__mapping_0)] calls=[hex_bits_2_backwards(__mapping_0)] inner=ta @ sew => e"
                    .into()
            )]
        );
    }

    #[test]
    fn forwards_direction_uses_forwards_names() {
        let body = arm_body(vec![
            Def::new(
                "f",
                DefKind::Function,
                vec![Stmt::Expr("match_arm ta_flag/forwards(sew) => e".into())],
            ),
            table("ta_flag"),
        ]);
        assert_eq!(
            body,
            vec![Stmt::Expr(
                "match_elab vars=[__mapping_0] guards=[ta_flag_forwards_matches(__mapping_0)] calls=[ta_flag_forwards(__mapping_0)] inner=sew => e"
                    .into()
            )]
        );
    }

    #[test]
    fn plain_arms_stay_and_original_guard_is_kept() {
        let body = arm_body(vec![
            Def::new(
                "f",
                DefKind::Function,
                vec![
                    Stmt::Expr("match_arm BYTE => e1".into()),
                    Stmt::Expr("match_arm hex_bits/backwards(s) when ok(s) => e2".into()),
                    Stmt::Expr("match_arm _ => e3".into()),
                ],
            ),
            table("hex_bits"),
        ]);
        assert_eq!(body[0], Stmt::Expr("match_arm BYTE => e1".into()));
        assert_eq!(body[2], Stmt::Expr("match_arm _ => e3".into()));
        assert_eq!(
            body[1],
            Stmt::Expr(
                "match_elab vars=[__mapping_0] guards=[hex_bits_backwards_matches(__mapping_0)] calls=[hex_bits_backwards(__mapping_0)] inner=s when ok(s) => e2"
                    .into()
            )
        );
    }

    #[test]
    fn unknown_mapping_malformed_and_elaborated_lines_pass_through() {
        let keep = vec![
            Stmt::Expr("match_arm nope/backwards(s) => e".into()),
            Stmt::Expr("match_arm hex_bits_2/backwards(unclosed => e".into()),
            Stmt::Expr("match_arm hex_bits_2/backwards(s)".into()),
            Stmt::Expr("match_elab vars=[__mapping_0] guards=[] => e".into()),
            Stmt::Expr("plain".into()),
        ];
        let body = arm_body(vec![
            Def::new("f", DefKind::Function, keep.clone()),
            table("hex_bits_2"),
        ]);
        assert_eq!(body, keep);
    }

    #[test]
    fn fresh_names_avoid_capture_nested_scopes_and_idempotency() {
        let def = Def::new(
            "f",
            DefKind::Function,
            vec![
                Stmt::Expr("__mapping_0".into()),
                Stmt::Expr("match_arm (hex_bits/backwards(a), ta_flag/forwards(b)) => e".into()),
                Stmt::LetTuple {
                    bindings: vec![],
                    rhs: "eff()".into(),
                    body: vec![Stmt::Expr("match_arm hex_bits/backwards(c) => e".into())],
                },
            ],
        );
        let out = arm_body(vec![def, table("hex_bits"), table("ta_flag")]);
        assert_eq!(out[0], Stmt::Expr("__mapping_0".into()));
        // `__mapping_0` is taken, so numbering starts at 1.
        let Stmt::Expr(first) = &out[1] else {
            panic!("elaborated")
        };
        assert!(first.contains("vars=[__mapping_1, __mapping_2]"), "{first}");
        assert!(first.contains("inner=(a, b)"), "{first}");
        let Stmt::LetTuple { body, .. } = &out[2] else {
            panic!("scope kept")
        };
        assert!(matches!(&body[0], Stmt::Expr(line) if line.starts_with("match_elab ")));
        let again = MappingPatterns
            .run(shard_with(vec![
                Def::new("f", DefKind::Function, out.clone()),
                table("hex_bits"),
                table("ta_flag"),
            ]))
            .defs
            .swap_remove(0)
            .body;
        assert_eq!(again, out, "pass must be idempotent");
    }
}
