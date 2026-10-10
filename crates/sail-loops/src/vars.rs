#![forbid(unsafe_code)]
//! `loop_vars`-Verwaltung: `make_loop_vars` + `name_loop_vars`.
//!
//! Ports (cf. `pretty_print_lean.ml`):
//!
//! ```ocaml
//! let make_loop_vars ctx extra_binders varstuple =
//!   match varstuple with
//!   | E_aux (E_tuple vs, _) ->
//!       let vs = List.map (doc_loop_var ctx) vs in
//!       let mkpp f vs = separate (string ", ") (List.map f vs) in
//!       let tup_pp = mkpp (fun (pp, _) -> pp) vs in
//!       let match_pp = mkpp (fun (_, pp) -> pp) vs in
//!       (parens tup_pp, separate space ((string "λ" :: extra_binders) @ [parens match_pp; string "=>"]))
//!   | _ ->
//!       let exp_pp, match_pp = doc_loop_var ctx varstuple in
//!       (exp_pp, separate space ((string "λ" :: extra_binders) @ [match_pp; string "=>"]))
//!
//! let name_loop_vars ctx =
//!   let ll = ctx.loop_level in
//!   ctx.loop_level <- ll + 1;
//!   match ll with 0 -> (string "loop_vars", ctx) | _ -> (string "loop_vars_" ^^ string (string_of_int ll), ctx)
//! ```
//!
//! `doc_loop_var` maps a variable to an `(expression, match-pattern)` pair;
//! for a plain id both are the id, for unit `()` they are `()` and `_`
//! (cf. `while_loopmon`: `let mut loop_vars := ()` … `(λ _ => …)` …
//! `let () := loop_vars`).

/// One loop-carried variable: how it is written as an expression
/// (`tup_pp` element) and how it is matched back (`match_pp` element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopVar {
    /// Expression form, e.g. `res` or `()`.
    pub expr: String,
    /// Match form, e.g. `res` or `_`.
    pub binder: String,
}

impl LoopVar {
    /// Plain identifier: expression and match form coincide.
    pub fn id(name: impl Into<String>) -> Self {
        let name = name.into();
        Self { expr: name.clone(), binder: name }
    }

    /// Unit state (`while` over registers only): `()` as expression,
    /// `_` as match.
    pub fn unit() -> Self {
        Self { expr: "()".to_owned(), binder: "_".to_owned() }
    }
}

/// The loop-carried state tuple (`varstuple` in OCaml).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopVars {
    /// Single variable (non-tuple `varstuple`).
    Single(LoopVar),
    /// Tuple of variables (`E_tuple`).
    Tuple(Vec<LoopVar>),
}

impl LoopVars {
    /// `vartuple_pp`: the initial-state expression
    /// (`res`, `(mult, res)`, `()`).
    pub fn init_expr(&self) -> String {
        match self {
            LoopVars::Single(v) => v.expr.clone(),
            LoopVars::Tuple(vs) => {
                let inner: Vec<&str> = vs.iter().map(|v| v.expr.as_str()).collect();
                format!("({})", inner.join(", "))
            }
        }
    }

    /// The `let`-pattern rebound from `loop_vars` each iteration
    /// (`let (mult, res) := loop_vars`).
    pub fn match_pattern(&self) -> String {
        match self {
            LoopVars::Single(v) => v.binder.clone(),
            LoopVars::Tuple(vs) => {
                let inner: Vec<&str> = vs.iter().map(|v| v.binder.as_str()).collect();
                format!("({})", inner.join(", "))
            }
        }
    }

    /// The `λ`-binder applied to the condition
    /// (`λ res => …`, `λ (mult, res) => …`, `λ _ => …`;
    /// `foreach#` prepends the index variable: `λ i res => …`).
    pub fn lambda(&self, extra_binders: &[&str]) -> String {
        let mut parts = vec!["λ"];
        parts.extend(extra_binders.iter().copied());
        let own = self.match_pattern();
        parts.push(own.as_str());
        parts.push("=>");
        parts.join(" ")
    }
}

/// Rendering context carrying the `loop_level` counter.
///
/// The counter starts at 0 per function and is bumped by every loop, so
/// nested loops get distinct state names (`loop_vars`, `loop_vars_1`, …)
/// and can never collide — including with user variables, which never use
/// the `loop_vars` prefix (enforced upstream by the renamer).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoopCtx {
    loop_level: usize,
}

impl LoopCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current nesting level (0 before the first loop).
    pub fn level(&self) -> usize {
        self.loop_level
    }

    /// Allocate the state-variable name for the next loop
    /// (`name_loop_vars` in OCaml).
    pub fn fresh_vars(&mut self) -> String {
        let level = self.loop_level;
        self.loop_level += 1;
        if level == 0 {
            "loop_vars".to_owned()
        } else {
            format!("loop_vars_{level}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_var_init_and_pattern() {
        let v = LoopVars::Single(LoopVar::id("res"));
        assert_eq!(v.init_expr(), "res");
        assert_eq!(v.match_pattern(), "res");
        assert_eq!(v.lambda(&[]), "λ res =>");
    }

    #[test]
    fn tuple_var_init_and_pattern() {
        // Order is the ANF varstuple order, cf. (mult, res) in loop.expected.lean.
        let v = LoopVars::Tuple(vec![LoopVar::id("mult"), LoopVar::id("res")]);
        assert_eq!(v.init_expr(), "(mult, res)");
        assert_eq!(v.match_pattern(), "(mult, res)");
        assert_eq!(v.lambda(&[]), "λ (mult, res) =>");
    }

    #[test]
    fn unit_var_splits_expr_and_binder() {
        let v = LoopVars::Single(LoopVar::unit());
        assert_eq!(v.init_expr(), "()");
        assert_eq!(v.match_pattern(), "_");
        assert_eq!(v.lambda(&[]), "λ _ =>");
    }

    #[test]
    fn foreach_lambda_carries_index_binder() {
        let v = LoopVars::Single(LoopVar::id("res"));
        assert_eq!(v.lambda(&["i"]), "λ i res =>");
        let t = LoopVars::Tuple(vec![LoopVar::id("mult"), LoopVar::id("res")]);
        assert_eq!(t.lambda(&["rr"]), "λ rr (mult, res) =>");
    }

    #[test]
    fn fresh_names_avoid_collisions() {
        let mut ctx = LoopCtx::new();
        assert_eq!(ctx.fresh_vars(), "loop_vars");
        assert_eq!(ctx.fresh_vars(), "loop_vars_1");
        assert_eq!(ctx.fresh_vars(), "loop_vars_2");
        assert_eq!(ctx.level(), 3);
    }

    #[test]
    fn nested_loops_get_distinct_names() {
        // foreachnotsopure: outer rr-loop vs inner i-loop.
        let mut ctx = LoopCtx::new();
        let outer = ctx.fresh_vars();
        let inner = ctx.fresh_vars();
        assert_ne!(outer, inner);
    }
}
