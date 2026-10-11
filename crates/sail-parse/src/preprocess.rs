//! Ordered preprocessing matching OCaml `preprocess.ml`.
//!
//! Macro state is explicit and survives files/includes in compilation order.
//! Include loading and definition consumption are injected; callers can spill
//! emitted definitions rather than accumulate a whole-program AST. Returning
//! `None` from the loader preserves an unresolved include pragma for audits;
//! a compiler must reject any such unresolved includes before emission.
#![forbid(unsafe_code)]

use crate::{iter_file, Def, PragmaArg, Span, SpannedDef};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct PreprocessError {
    pub path: String,
    pub span: Span,
    pub message: String,
}
impl std::fmt::Display for PreprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.span, self.message)
    }
}
impl std::error::Error for PreprocessError {}

pub struct Preprocessor {
    target: Option<String>,
    symbols: BTreeSet<String>,
    target_sets: BTreeMap<String, Vec<String>>,
}
struct Conditional {
    span: Span,
    parent: bool,
    condition: bool,
    seen_else: bool,
}
impl Preprocessor {
    pub fn new(target: Option<String>) -> Self {
        Self {
            target,
            symbols: [
                "FEATURE_IMPLICITS",
                "FEATURE_CONSTANT_TYPES",
                "FEATURE_BITVECTOR_TYPE",
                "FEATURE_UNION_BARRIER",
                "FEATURE_STRICT_VAR",
                "FEATURE_STRICT_BITVECTOR",
                "FEATURE_STRICT_EXPONENTIALS",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            target_sets: BTreeMap::new(),
        }
    }
    pub fn define(&mut self, symbol: impl Into<String>) {
        self.symbols.insert(symbol.into());
    }

    /// Process one file, expanding includes at the exact directive position.
    /// `load(includer, raw_argument)` receives `"relative.sail"` or
    /// `<library.sail>` and returns the resolved path and source, or `None`
    /// to retain the include explicitly. `emit` receives definitions in order,
    /// with their original path and byte span. Errors stop immediately.
    pub fn process<L, S>(
        &mut self,
        path: &str,
        source: &str,
        mut load: L,
        mut emit: S,
    ) -> Result<(), PreprocessError>
    where
        L: FnMut(&str, &str) -> Result<Option<(String, String)>, String>,
        S: FnMut(&str, SpannedDef) -> Result<(), String>,
    {
        self.process_inner(path, source, &mut load, &mut emit, 0)
    }

    fn process_inner(
        &mut self,
        path: &str,
        source: &str,
        load: &mut dyn FnMut(&str, &str) -> Result<Option<(String, String)>, String>,
        emit: &mut dyn FnMut(&str, SpannedDef) -> Result<(), String>,
        depth: usize,
    ) -> Result<(), PreprocessError> {
        let error = |span, message| PreprocessError {
            path: path.into(),
            span,
            message,
        };
        if depth >= 256 {
            return Err(error(
                Span::DUMMY,
                "include nesting limit exceeded (possible unguarded cycle)".into(),
            ));
        }
        let mut stack: Vec<Conditional> = Vec::new();
        let mut active = true;
        for def in iter_file(path, source) {
            let def = def.map_err(|e| error(e.span, e.message))?;
            let span = def.span;
            if let Def::Pragma {
                name,
                arg: PragmaArg::Line(arg),
            } = &def.def
            {
                match name.as_str() {
                    "ifdef" | "ifndef" | "iftarget" => {
                        let condition = match name.as_str() {
                            "ifdef" => self.symbols.contains(arg),
                            "ifndef" => !self.symbols.contains(arg),
                            _ => self.target.as_ref().is_some_and(|target| {
                                arg.split_whitespace().any(|name| {
                                    name == target
                                        || self
                                            .target_sets
                                            .get(name)
                                            .is_some_and(|set| set.contains(target))
                                })
                            }),
                        };
                        stack.push(Conditional {
                            span,
                            parent: active,
                            condition,
                            seen_else: false,
                        });
                        active &= condition;
                        continue;
                    }
                    "else" => {
                        let top = stack
                            .last_mut()
                            .ok_or_else(|| error(span, "$else without conditional".into()))?;
                        if top.seen_else {
                            return Err(error(span, "duplicate $else".into()));
                        }
                        top.seen_else = true;
                        active = top.parent && !top.condition;
                        continue;
                    }
                    "endif" => {
                        let top = stack
                            .pop()
                            .ok_or_else(|| error(span, "$endif without conditional".into()))?;
                        active = top.parent;
                        continue;
                    }
                    _ if !active => continue,
                    "file_start" | "file_end" => continue,
                    "define" => {
                        self.define(arg.clone());
                        continue;
                    }
                    "include_error" => return Err(error(span, arg.clone())),
                    "target_set" => {
                        let mut words = arg.split_whitespace();
                        let name = words
                            .next()
                            .ok_or_else(|| error(span, "empty $target_set".into()))?;
                        self.target_sets
                            .insert(name.into(), words.map(str::to_owned).collect());
                    }
                    "include" => {
                        if let Some((included_path, included_source)) =
                            load(path, arg).map_err(|e| error(span, e))?
                        {
                            let marker = |name: &str| {
                                SpannedDef::new(
                                    Def::Pragma {
                                        name: name.into(),
                                        arg: PragmaArg::Line(included_path.clone()),
                                    },
                                    span,
                                )
                            };
                            let mut started = false;
                            self.process_inner(
                                &included_path,
                                &included_source,
                                load,
                                &mut |origin, def| {
                                    if !started {
                                        emit(path, marker("include_start"))?;
                                        started = true;
                                    }
                                    emit(origin, def)
                                },
                                depth + 1,
                            )?;
                            if started {
                                emit(path, marker("include_end")).map_err(|e| error(span, e))?;
                            }
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            if active {
                if let Def::Default { kind, value } = &def.def {
                    if kind == "Order" {
                        self.define("_DEFAULT_ORDER_SET");
                        match value.as_str() {
                            "inc" => self.define("_DEFAULT_INC"),
                            "dec" => self.define("_DEFAULT_DEC"),
                            _ => {}
                        }
                    }
                }
                emit(path, def).map_err(|e| error(span, e))?;
            }
        }
        if let Some(top) = stack.first() {
            return Err(error(top.span, "conditional never ended by $endif".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn names(src: &str) -> Result<Vec<String>, PreprocessError> {
        let mut names = Vec::new();
        Preprocessor::new(Some("lean".into())).process(
            "test.sail",
            src,
            |_, _| Ok(None),
            |_, def| {
                if let Def::Val { name, .. } = def.def {
                    names.push(name);
                }
                Ok(())
            },
        )?;
        Ok(names)
    }
    #[test]
    fn selects_nested_branches_and_target_sets() {
        assert_eq!(names("$target_set prover lean coq\n$ifdef ABSENT\n$define LEAK\n$else\n$iftarget prover\nval yes : int\n$endif\n$endif\n$ifdef LEAK\nval bad : int\n$endif\n").unwrap(), ["yes"]);
    }
    #[test]
    fn include_defines_affect_following_parent_definitions() {
        let mut names = Vec::new();
        Preprocessor::new(Some("lean".into())).process("parent.sail",
            "$include <guard.sail>\n$ifdef INCLUDED\nval after : int\n$endif\n$include <guard.sail>\n",
            |_, arg| { assert_eq!(arg, "<guard.sail>"); Ok(Some(("guard.sail".into(),
                "$ifndef INCLUDED\n$define INCLUDED\nval inside : int\n$endif\n".into()))) },
            |path, def| { if let Def::Val { name, .. } = def.def { names.push((path.to_string(), name)); } Ok(()) }
        ).unwrap();
        assert_eq!(
            names,
            [
                ("guard.sail".into(), "inside".into()),
                ("parent.sail".into(), "after".into())
            ]
        );
    }
    #[test]
    fn rejects_malformed_conditionals_and_ignores_inactive_errors() {
        for src in [
            "$else\n",
            "$endif\n",
            "$ifdef X\n",
            "$ifdef X\n$else\n$else\n$endif\n",
        ] {
            assert!(names(src).is_err(), "accepted {src}");
        }
        assert!(names("$ifdef ABSENT\n$include_error skipped\n$endif\n").is_ok());
        assert!(names("$include_error fail\n").is_err());
    }
    #[test]
    fn directives_in_comments_and_strings_have_no_effect() {
        assert_eq!(names("/*\n$define X\n*/\nval s : string\nfunction s() = \"$define X\"\n$ifdef X\nval bad : int\n$endif\n").unwrap(), ["s"]);
    }
    #[test]
    fn default_order_defines_ocaml_feature_symbols() {
        assert_eq!(names("default Order dec\n$ifdef _DEFAULT_ORDER_SET\n$ifdef _DEFAULT_DEC\nval yes : int\n$endif\n$endif\n$ifdef _DEFAULT_INC\nval bad : int\n$endif\n").unwrap(), ["yes"]);
    }
    #[test]
    fn wraps_nonempty_includes_only_and_keeps_origin_spans() {
        let mut out = Vec::new();
        Preprocessor::new(None)
            .process(
                "parent",
                "$include <a>\n$include <empty>\n",
                |_, arg| {
                    Ok(Some((
                        arg.into(),
                        if arg == "<a>" {
                            "val x : int\n".into()
                        } else {
                            String::new()
                        },
                    )))
                },
                |path, def| {
                    out.push((path.to_string(), def));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, "parent");
        assert_eq!(out[1].0, "<a>");
        assert_eq!(out[2].0, "parent");
        assert!(matches!(&out[0].1.def, Def::Pragma { name, .. } if name == "include_start"));
        assert!(matches!(&out[2].1.def, Def::Pragma { name, .. } if name == "include_end"));
    }
}
