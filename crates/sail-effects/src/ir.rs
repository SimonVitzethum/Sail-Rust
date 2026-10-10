//! Minimal test IR for effect analysis.
//!
//! This is deliberately small: Lane 08 must not depend on the full
//! Sail frontend (other lanes). Lane 10 maps the real AST onto these
//! predicates; the shapes here mirror what Lane 10 needs for `do`
//! blocks / `<-` (bind) lowering.

#![forbid(unsafe_code)]

/// A whole function definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunDef {
    /// Function name (unique within a program).
    pub name: String,
    /// Body as a flat block.
    pub body: Block,
}

impl FunDef {
    pub fn new(name: impl Into<String>, body: Block) -> Self {
        Self {
            name: name.into(),
            body,
        }
    }
}

/// A sequence of statements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

impl Block {
    pub fn new(stmts: Vec<Stmt>) -> Self {
        Self { stmts }
    }

    pub fn empty() -> Self {
        Self { stmts: Vec::new() }
    }
}

/// Statements. Variants are kept minimal but cover what the effect
/// predicates need: pure control flow, loops, early returns /
/// exceptions, and machine effects (register / memory / nondeterminism).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `let x = expr`
    Let { name: String, expr: Expr },
    /// `x = expr` (local assignment, pure).
    Assign { name: String, expr: Expr },
    /// Register write: `reg <- expr` — a `SailM` effect.
    WriteReg { reg: String, expr: Expr },
    /// Memory write — a `SailM` effect.
    WriteMem { addr: Expr, value: Expr },
    /// Expression statement (e.g. a call whose value is discarded).
    ExprStmt(Expr),
    /// `if c then a else b`
    If {
        cond: Expr,
        then_branch: Block,
        else_branch: Block,
    },
    /// `while c do body`
    While { cond: Expr, body: Block },
    /// `for i in lo..hi do body` (desugared loop).
    For {
        var: String,
        lo: Expr,
        hi: Expr,
        body: Block,
    },
    /// Tail-position return: `return expr`.
    Return(Expr),
    /// `throw expr` / `exit` / `assert(false)`: exceptional control flow.
    /// Counts as early return AND as an effect (`ExceptM`).
    Throw(Expr),
    /// `assert(cond)`: pure check, but compiled via `ExceptM` when it
    /// can fail; treated as early-return-ish, not as `SailM`.
    Assert(Expr),
}

/// Expressions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// Integer / bool / string literal.
    Lit(i64),
    /// Variable reference.
    Var(String),
    /// Register read — a `SailM` effect.
    ReadReg(String),
    /// Memory read — a `SailM` effect.
    ReadMem(Box<Expr>),
    /// Function call `f(args)`. Purity depends on the callee.
    Call { func: String, args: Vec<Expr> },
    /// `choose(...)` / nondeterministic pick — a `SailM` effect.
    Nondet,
    /// Binary op over pure subexpressions (`a + b`, `a == b`, ...).
    Binop {
        op: Binop,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

/// Pure binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binop {
    Add,
    Sub,
    Eq,
    Lt,
}
