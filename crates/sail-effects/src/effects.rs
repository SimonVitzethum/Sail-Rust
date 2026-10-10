//! Pure effect predicates over [`crate::ir`].
//!
//! All functions are pure: they borrow the AST and allocate nothing
//! except the returned value. They never consult global state, so
//! callers can run them on many functions in parallel.

#![forbid(unsafe_code)]

use crate::ir::{Block, Expr, FunDef, Stmt};

/// Coarse effect label, mirroring the OCaml backend:
/// - `Pure`: no register/memory/nondet effects, no `throw`.
/// - `ExceptM`: may early-return / throw (`throw`, `assert`, non-tail
///   `return`). No register/memory effects.
/// - `SailM`: may touch registers / memory / nondeterminism
///   (transitively the strongest label).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectLabel {
    Pure,
    ExceptM,
    SailM,
}

/// True if the expression itself performs a machine effect
/// (register / memory read, nondeterminism). Calls are *not* counted
/// here: use the callgraph for transitive reasoning.
pub fn expr_has_direct_effect(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) | Expr::Var(_) => false,
        Expr::ReadReg(_) | Expr::ReadMem(_) | Expr::Nondet => true,
        Expr::Call { .. } => false,
        Expr::Binop { lhs, rhs, .. } => expr_has_direct_effect(lhs) || expr_has_direct_effect(rhs),
    }
}

/// True if the statement itself performs a machine effect
/// (`WriteReg` / `WriteMem`, or an effectful subexpression).
/// Control-flow statements recurse into their sub-blocks.
pub fn has_effect(fun: &FunDef) -> bool {
    block_has_effect(&fun.body)
}

/// True if the function contains any loop (`while` / `for`).
pub fn has_loop(fun: &FunDef) -> bool {
    block_has_loop(&fun.body)
}

/// True if the function can exit before its end:
/// - any `throw` / `assert` anywhere, or
/// - any `return` that is not the single trailing statement of the
///   outermost body.
///
/// A trailing `return v` (or no `return` at all, i.e. fall-through)
/// is the normal Lean `return`, not an early return.
pub fn has_early_return(fun: &FunDef) -> bool {
    let stmts = &fun.body.stmts;
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i + 1 == stmts.len();
        if stmt_has_early_return(stmt, is_last) {
            return true;
        }
    }
    false
}

/// True if the function is directly pure: no machine effects and no
/// `throw`. (Calls to other functions are not resolved here; use the
/// callgraph + [`effect_label`]’s transitive helpers in Lane 10, or
/// check the callee labels manually.)
///
/// Note: loops and early returns do not make a function impure by
/// themselves — they affect the `ExceptM` label, see [`effect_label`].
pub fn function_is_pure(fun: &FunDef) -> bool {
    !has_effect(fun) && !block_has_throw(&fun.body)
}

/// Combined label for one function body (direct effects only;
/// callers compose transitively via the callgraph).
pub fn effect_label(fun: &FunDef) -> EffectLabel {
    if has_effect(fun) {
        EffectLabel::SailM
    } else if has_early_return(fun) {
        EffectLabel::ExceptM
    } else {
        EffectLabel::Pure
    }
}

// --- helpers (all pure, borrow only) ---

fn block_has_effect(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_effect)
}

fn stmt_has_effect(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::WriteReg { .. } | Stmt::WriteMem { .. } => true,
        Stmt::Let { expr, .. } | Stmt::Assign { expr, .. } | Stmt::Return(expr) | Stmt::Throw(expr) | Stmt::Assert(expr) | Stmt::ExprStmt(expr) => {
            expr_has_direct_effect(expr)
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => expr_has_direct_effect(cond) || block_has_effect(then_branch) || block_has_effect(else_branch),
        Stmt::While { cond, body } => expr_has_direct_effect(cond) || block_has_effect(body),
        Stmt::For { lo, hi, body, .. } => expr_has_direct_effect(lo) || expr_has_direct_effect(hi) || block_has_effect(body),
    }
}

fn block_has_loop(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_loop)
}

fn stmt_has_loop(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::While { .. } | Stmt::For { .. } => true,
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => block_has_loop(then_branch) || block_has_loop(else_branch),
        _ => false,
    }
}

fn block_has_throw(block: &Block) -> bool {
    block.stmts.iter().any(|s| match s {
        Stmt::Throw(_) => true,
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => block_has_throw(then_branch) || block_has_throw(else_branch),
        Stmt::While { body, .. } | Stmt::For { body, .. } => block_has_throw(body),
        _ => false,
    })
}

/// `is_last` = whether this statement is the trailing statement of the
/// outermost function body. A `Return` in tail position is normal;
/// anywhere else (including inside nested blocks) it is early.
fn stmt_has_early_return(stmt: &Stmt, is_last: bool) -> bool {
    match stmt {
        Stmt::Return(_) => !is_last,
        Stmt::Throw(_) | Stmt::Assert(_) => true,
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => block_has_any_early_return(then_branch) || block_has_any_early_return(else_branch),
        Stmt::While { body, .. } | Stmt::For { body, .. } => block_has_any_early_return(body),
        _ => false,
    }
}

/// Any `return`/`throw`/`assert` at any position inside the block.
fn block_has_any_early_return(block: &Block) -> bool {
    block.stmts.iter().any(|s| match s {
        Stmt::Return(_) | Stmt::Throw(_) | Stmt::Assert(_) => true,
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => block_has_any_early_return(then_branch) || block_has_any_early_return(else_branch),
        Stmt::While { body, .. } | Stmt::For { body, .. } => block_has_any_early_return(body),
        _ => false,
    })
}
