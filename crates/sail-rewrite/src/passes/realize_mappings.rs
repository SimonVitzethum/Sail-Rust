//! `realize_mappings` over the string-statement shard AST.
//!
//! Source: `sail/src/lib/rewrites.ml` (`rewrite_ast_realize_mappings`).
//! An OCaml mapping
//!
//! ```sail
//! val size_bits : word_width <-> bits(2)
//! mapping size_bits = { BYTE <-> 0b00, ... }
//! ```
//!
//! elaborates into four functions (`size_bits_forwards`,
//! `size_bits_backwards`, `size_bits_forwards_matches`,
//! `size_bits_backwards_matches`); a bidirectional `val` spec splits into
//! four function specs; `overload` entries naming mappings expand to the
//! `_forwards`/`_backwards` pair.
//!
//! The string [`Shard`] surface has no mapping constructs, so this pass
//! works on explicit marker lines (all other definitions pass through
//! untouched):
//!
//! - Mapping source: `Def { kind: Other, body: ["mapping <m> : <t1> <-> <t2>",
//!   <clause>, ...] }`, where each clause is one of
//!   `bidir <lhs> [when <g1>] <-> <rhs> [when <g2>]`,
//!   `forwards <pat> [when <g>] => <exp>`,
//!   `backwards <pat> [when <g>] => <exp>`.
//! - Bidir val spec: `Def { kind: Valspec, body: ["bidir_spec <t1> <-> <t2>"] }`.
//! - Overload: any `Other` def holding `Stmt::Expr("overload <o> = <e...>")
//!   lines; entries naming a same-shard mapping expand to
//!   `<e>_forwards <e>_backwards`.
//!
//! Guards travel with their pattern side (`MCL_bidir` semantics). The last
//! arm of each direction carries ` #last` (OCaml `mapping_last`); the
//! `_matches` variants replace every body with `true` and append a wildcard
//! `arm _ => false`. Functions that already exist in the shard are not
//! regenerated (`has_def`); the mapping source is consumed in any case.
//!
//! Shard-local (mapping names come from the same shard only), deterministic
//! (clause order preserved, `*_forwards` before `*_backwards` before
//! `*_matches`), and idempotent (generated defs carry no mapping markers).

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use crate::ast::{Def, DefKind, Shard, Stmt};
use crate::pass::Pass;

#[derive(Clone, Copy, Debug, Default)]
pub struct RealizeMappings;

impl Pass for RealizeMappings {
    fn name(&self) -> &'static str {
        "realize_mappings"
    }

    fn run(&self, shard: Shard) -> Shard {
        realize(shard)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Clause {
    Bidir {
        lhs: String,
        left_guard: Option<String>,
        rhs: String,
        right_guard: Option<String>,
    },
    Forwards {
        pat: String,
        guard: Option<String>,
        body: String,
    },
    Backwards {
        pat: String,
        guard: Option<String>,
        body: String,
    },
}

fn split_once(haystack: &str, needle: &str) -> Option<(String, String)> {
    let idx = haystack.find(needle)?;
    let (left, right) = haystack.split_at(idx);
    Some((left.to_string(), right[needle.len()..].to_string()))
}

fn split_pat_guard(part: &str) -> Option<(String, Option<String>)> {
    let part = part.trim();
    if part.is_empty() {
        return None;
    }
    match split_once(part, " when ") {
        Some((pat, guard)) => {
            let (pat, guard) = (pat.trim(), guard.trim());
            if pat.is_empty() || guard.is_empty() {
                None
            } else {
                Some((pat.to_string(), Some(guard.to_string())))
            }
        }
        None => Some((part.to_string(), None)),
    }
}

fn parse_mapping_header(def_name: &str, line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("mapping ")?.trim();
    let (name_part, types) = split_once(rest, ":")?;
    if name_part.trim() != def_name {
        return None;
    }
    let (t1, t2) = split_once(&types, "<->")?;
    let (t1, t2) = (t1.trim(), t2.trim());
    if t1.is_empty() || t2.is_empty() {
        return None;
    }
    Some((t1.to_string(), t2.to_string()))
}

fn parse_clause(line: &str) -> Option<Clause> {
    if let Some(rest) = line.strip_prefix("bidir ") {
        let (left, right) = split_once(rest, "<->")?;
        let (lhs, left_guard) = split_pat_guard(&left)?;
        let (rhs, right_guard) = split_pat_guard(&right)?;
        return Some(Clause::Bidir {
            lhs,
            left_guard,
            rhs,
            right_guard,
        });
    }
    for (prefix, is_fwd) in [("forwards ", true), ("backwards ", false)] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let (left, body) = split_once(rest, "=>")?;
            let (pat, guard) = split_pat_guard(&left)?;
            let body = body.trim();
            if body.is_empty() {
                return None;
            }
            let (pat, guard, body) = (pat, guard, body.to_string());
            return Some(if is_fwd {
                Clause::Forwards { pat, guard, body }
            } else {
                Clause::Backwards { pat, guard, body }
            });
        }
    }
    None
}

fn parse_bidir_spec(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("bidir_spec ")?;
    let (t1, t2) = split_once(rest, "<->")?;
    let (t1, t2) = (t1.trim(), t2.trim());
    if t1.is_empty() || t2.is_empty() {
        return None;
    }
    Some((t1.to_string(), t2.to_string()))
}

fn is_mapping_def(def: &Def) -> bool {
    if def.kind != DefKind::Other {
        return false;
    }
    let [Stmt::Expr(header), ..] = def.body.as_slice() else {
        return false;
    };
    parse_mapping_header(&def.name, header.trim()).is_some()
}

fn is_bidir_valspect(def: &Def) -> bool {
    if def.kind != DefKind::Valspec {
        return false;
    }
    let [Stmt::Expr(line)] = def.body.as_slice() else {
        return false;
    };
    parse_bidir_spec(line.trim()).is_some()
}

fn format_arm(pat: &str, guard: &Option<String>, body: &str, is_last: bool) -> Stmt {
    let mut text = format!("arm {pat}");
    if let Some(guard) = guard {
        text.push_str(&format!(" when {guard}"));
    }
    text.push_str(&format!(" => {body}"));
    if is_last {
        text.push_str(" #last");
    }
    Stmt::Expr(text)
}

fn realize(shard: Shard) -> Shard {
    let mut mapping_names: BTreeSet<String> = BTreeSet::new();
    for def in &shard.defs {
        if is_mapping_def(def) || is_bidir_valspect(def) {
            mapping_names.insert(def.name.clone());
        }
    }
    let mut existing: BTreeSet<String> = shard.defs.iter().map(|d| d.name.clone()).collect();
    let mut out: Vec<Def> = Vec::with_capacity(shard.defs.len());

    for def in shard.defs {
        if def.kind == DefKind::Other && is_mapping_def(&def) {
            if let Some(generated) = realize_mapping_def(&def, &mut existing) {
                out.extend(generated);
            } else {
                // Malformed clause lines: preserve the source verbatim rather
                // than silently dropping user code.
                out.push(def);
            }
            continue;
        }
        if def.kind == DefKind::Valspec {
            if let [Stmt::Expr(line)] = def.body.as_slice() {
                if let Some((t1, t2)) = parse_bidir_spec(line.trim()) {
                    for spec in realize_val_spec(&def.name, &t1, &t2) {
                        if existing.insert(spec.name.clone()) {
                            out.push(spec);
                        }
                    }
                    continue;
                }
            }
            out.push(def);
            continue;
        }
        if def.kind == DefKind::Other && def.body.iter().any(is_overload_line) {
            out.push(expand_overload(def, &mapping_names));
            continue;
        }
        out.push(def);
    }

    Shard::new(shard.id, out)
}

/// Realize one mapping def; `None` when a clause line is malformed (caller
/// keeps the source verbatim).
fn realize_mapping_def(def: &Def, existing: &mut BTreeSet<String>) -> Option<Vec<Def>> {
    let [Stmt::Expr(header), clauses @ ..] = def.body.as_slice() else {
        return None;
    };
    let (_t1, _t2) = parse_mapping_header(&def.name, header.trim())?;
    let mut parsed: Vec<Clause> = Vec::with_capacity(clauses.len());
    for stmt in clauses {
        let Stmt::Expr(line) = stmt else {
            return None;
        };
        parsed.push(parse_clause(line.trim())?);
    }

    let mut fwd: Vec<(String, Option<String>, String)> = Vec::new();
    let mut bwd: Vec<(String, Option<String>, String)> = Vec::new();
    for clause in &parsed {
        match clause {
            Clause::Bidir {
                lhs,
                left_guard,
                rhs,
                right_guard,
            } => {
                fwd.push((lhs.clone(), left_guard.clone(), rhs.clone()));
                bwd.push((rhs.clone(), right_guard.clone(), lhs.clone()));
            }
            Clause::Forwards { pat, guard, body } => {
                fwd.push((pat.clone(), guard.clone(), body.clone()));
            }
            Clause::Backwards { pat, guard, body } => {
                bwd.push((pat.clone(), guard.clone(), body.clone()));
            }
        }
    }

    let plain = |arms: &[(String, Option<String>, String)]| -> Vec<Stmt> {
        let n = arms.len();
        arms.iter()
            .enumerate()
            .map(|(i, (pat, guard, body))| format_arm(pat, guard, body, i + 1 == n))
            .collect()
    };
    let matches = |arms: &[(String, Option<String>, String)]| -> Vec<Stmt> {
        let n = arms.len();
        let mut body: Vec<Stmt> = arms
            .iter()
            .enumerate()
            .map(|(i, (pat, guard, _))| format_arm(pat, guard, "true", i + 1 == n))
            .collect();
        body.push(Stmt::Expr("arm _ => false".to_string()));
        body
    };

    let mut generated = Vec::new();
    let mut emit = |name: String, body: Vec<Stmt>| {
        if existing.insert(name.clone()) {
            generated.push(Def::new(name, DefKind::Function, body));
        }
    };
    emit(format!("{}_forwards", def.name), plain(&fwd));
    emit(format!("{}_backwards", def.name), plain(&bwd));
    emit(format!("{}_forwards_matches", def.name), matches(&fwd));
    emit(format!("{}_backwards_matches", def.name), matches(&bwd));
    Some(generated)
}

fn realize_val_spec(name: &str, t1: &str, t2: &str) -> Vec<Def> {
    vec![
        Def::new(
            format!("{name}_forwards"),
            DefKind::Valspec,
            vec![Stmt::Expr(format!("spec {t1} -> {t2}"))],
        ),
        Def::new(
            format!("{name}_backwards"),
            DefKind::Valspec,
            vec![Stmt::Expr(format!("spec {t2} -> {t1}"))],
        ),
        Def::new(
            format!("{name}_forwards_matches"),
            DefKind::Valspec,
            vec![Stmt::Expr(format!("spec {t1} -> bool"))],
        ),
        Def::new(
            format!("{name}_backwards_matches"),
            DefKind::Valspec,
            vec![Stmt::Expr(format!("spec {t2} -> bool"))],
        ),
    ]
}

fn is_overload_line(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Expr(line) if line.trim_start().starts_with("overload "))
}

fn expand_overload(mut def: Def, mapping_names: &BTreeSet<String>) -> Def {
    for stmt in &mut def.body {
        let Stmt::Expr(line) = stmt else {
            continue;
        };
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("overload ") else {
            continue;
        };
        let Some((name_part, entries_part)) = split_once(rest, "=") else {
            continue;
        };
        let name = name_part.trim();
        if name.is_empty() {
            continue;
        }
        let mut expanded: Vec<String> = Vec::new();
        for entry in entries_part.split_whitespace() {
            if mapping_names.contains(entry) {
                expanded.push(format!("{entry}_forwards"));
                expanded.push(format!("{entry}_backwards"));
            } else {
                expanded.push(entry.to_string());
            }
        }
        *line = format!("overload {name} = {}", expanded.join(" "));
    }
    def
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping_def() -> Def {
        Def::new(
            "size_bits",
            DefKind::Other,
            vec![
                Stmt::Expr("mapping size_bits : word_width <-> bits(2)".into()),
                Stmt::Expr("bidir BYTE <-> 0b00".into()),
                Stmt::Expr("bidir HALF <-> 0b01".into()),
                Stmt::Expr("bidir WORD <-> 0b10".into()),
                Stmt::Expr("bidir DOUBLE <-> 0b11".into()),
            ],
        )
    }

    fn run(defs: Vec<Def>) -> Vec<Def> {
        RealizeMappings.run(Shard::new(0, defs)).defs
    }

    #[test]
    fn bidir_mapping_realizes_four_functions() {
        let out = run(vec![mapping_def()]);
        assert_eq!(
            out.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec![
                "size_bits_forwards",
                "size_bits_backwards",
                "size_bits_forwards_matches",
                "size_bits_backwards_matches",
            ]
        );
        assert!(out.iter().all(|d| d.kind == DefKind::Function));
        assert_eq!(out[0].body[0], Stmt::Expr("arm BYTE => 0b00".into()));
        assert_eq!(
            out[0].body[3],
            Stmt::Expr("arm DOUBLE => 0b11 #last".into())
        );
        assert_eq!(out[1].body[0], Stmt::Expr("arm 0b00 => BYTE".into()));
        assert_eq!(
            out[2].body[..4],
            [
                Stmt::Expr("arm BYTE => true".into()),
                Stmt::Expr("arm HALF => true".into()),
                Stmt::Expr("arm WORD => true".into()),
                Stmt::Expr("arm DOUBLE => true #last".into()),
            ]
        );
        assert_eq!(out[2].body[4], Stmt::Expr("arm _ => false".into()));
        assert_eq!(out[3].body.len(), 5);
    }

    #[test]
    fn direction_only_clauses_are_filtered() {
        let mut def = mapping_def();
        def.name = "size_bits3".to_string();
        def.body[0] = Stmt::Expr("mapping size_bits3 : word_width <-> bits(2)".into());
        def.body.truncate(4);
        def.body.push(Stmt::Expr("forwards DOUBLE => 0b11".into()));
        def.body.push(Stmt::Expr("backwards 0b11 => DOUBLE".into()));
        let out = run(vec![def]);
        assert_eq!(out[0].body.len(), 4);
        assert_eq!(out[1].body.len(), 4);
        assert_eq!(
            out[0].body[3],
            Stmt::Expr("arm DOUBLE => 0b11 #last".into())
        );
        assert_eq!(
            out[1].body[3],
            Stmt::Expr("arm 0b11 => DOUBLE #last".into())
        );
    }

    #[test]
    fn guards_travel_with_pattern_side() {
        let out = run(vec![Def::new(
            "vtype_assembly",
            DefKind::Other,
            vec![
                Stmt::Expr("mapping vtype_assembly : string <-> (bits(1), bits(1))".into()),
                Stmt::Expr("bidir fuse(sew) ^ fuse(ta) when eq_bit(sew) <-> (ta, sew)".into()),
            ],
        )]);
        assert_eq!(
            out[0].body[0],
            Stmt::Expr("arm fuse(sew) ^ fuse(ta) when eq_bit(sew) => (ta, sew) #last".into())
        );
        assert_eq!(
            out[1].body[0],
            Stmt::Expr("arm (ta, sew) => fuse(sew) ^ fuse(ta) #last".into())
        );
    }

    #[test]
    fn bidir_val_spec_splits_into_four() {
        let out = run(vec![Def::new(
            "size_bits",
            DefKind::Valspec,
            vec![Stmt::Expr("bidir_spec word_width <-> bits(2)".into())],
        )]);
        assert_eq!(
            out.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec![
                "size_bits_forwards",
                "size_bits_backwards",
                "size_bits_forwards_matches",
                "size_bits_backwards_matches",
            ]
        );
        assert!(out.iter().all(|d| d.kind == DefKind::Valspec));
        assert_eq!(
            out[0].body,
            vec![Stmt::Expr("spec word_width -> bits(2)".into())]
        );
        assert_eq!(
            out[1].body,
            vec![Stmt::Expr("spec bits(2) -> word_width".into())]
        );
    }

    #[test]
    fn existing_definitions_are_not_regenerated_and_mapping_is_consumed() {
        let existing = Def::new("size_bits_forwards", DefKind::Function, vec![]);
        let out = run(vec![existing, mapping_def()]);
        assert_eq!(
            out.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec![
                "size_bits_forwards",
                "size_bits_backwards",
                "size_bits_forwards_matches",
                "size_bits_backwards_matches",
            ]
        );
        // The pre-existing def keeps its body; the mapping source is gone.
        assert_eq!(out[0].body, Vec::<Stmt>::new());
        assert!(!out.iter().any(|d| d.name == "size_bits"));
    }

    #[test]
    fn overload_entries_expand_mappings() {
        let out = run(vec![
            mapping_def(),
            Def::new(
                "ov",
                DefKind::Other,
                vec![Stmt::Expr("overload o = size_bits add".into())],
            ),
        ]);
        let ov = out.iter().find(|d| d.name == "ov").expect("overload kept");
        assert_eq!(
            ov.body,
            vec![Stmt::Expr(
                "overload o = size_bits_forwards size_bits_backwards add".into()
            )]
        );
    }

    #[test]
    fn malformed_and_foreign_defs_pass_through_and_pass_is_idempotent() {
        let foreign = Def::new("f", DefKind::Function, vec![Stmt::Expr("x".into())]);
        let bad = Def::new(
            "bad",
            DefKind::Other,
            vec![
                Stmt::Expr("mapping bad : t1 <-> t2".into()),
                Stmt::Expr("bidir only-one-side".into()),
            ],
        );
        let once = run(vec![foreign.clone(), bad.clone(), mapping_def()]);
        assert!(once.iter().any(|d| d.name == "f"));
        assert!(once.iter().any(|d| d.name == "bad"));
        let twice = RealizeMappings.run(Shard::new(0, once.clone())).defs;
        assert_eq!(twice, once);
    }
}
