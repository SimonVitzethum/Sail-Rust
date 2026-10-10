//! Minimale Test-IR für Lane 09.
//!
//! Absichtlich klein: genug Struktur für Mono-Splits (`type_params`),
//! ANF (`Let`/`Block`/verschachtelte Calls) und Exhaustiveness pro Funktion.
//! Echte Sail-AST kommt aus späteren Lanes; die Lowering-Pässe arbeiten
//! bewusst nur auf dieser IR (Skizze, kein OCaml-Parity hier).

/// Einfache Typen: Typvariablen für Mono, Rest konkret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Typvariable, z.B. `'a` aus polymorpher Signatur.
    Var(String),
    /// Konkreter benannter Typ (`int`, `bool`, `bits(n)`, ...).
    Named(String),
    /// `bits(n)`-Spezialfall für Split-Limit-Tests.
    Bits(u32),
}

impl Ty {
    pub fn is_var(&self) -> bool {
        matches!(self, Ty::Var(_))
    }
}

impl core::fmt::Display for Ty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Ty::Var(v) => write!(f, "'{v}"),
            Ty::Named(n) => write!(f, "{n}"),
            Ty::Bits(n) => write!(f, "bits({n})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lit {
    Int(i128),
    Bool(bool),
    Bits(String),
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Eq,
    And,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pat {
    Wildcard,
    Var(String),
    Lit(Lit),
    Tuple(Vec<Pat>),
    Ctor(String, Vec<Pat>),
}

impl Pat {
    /// Ob das Pattern alles abdeckt (Wildcard oder plain Variable).
    pub fn is_irrefutable(&self) -> bool {
        matches!(self, Pat::Wildcard | Pat::Var(_))
    }
}

/// Ausdrücke der Test-IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Var(String),
    Lit(Lit),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// `let pat = value in body`
    Let(Pat, Box<Expr>, Box<Expr>),
    /// `{ stmts...; tail }` — wird von `remove_blocks` eliminiert.
    Block(Vec<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Match(Box<Expr>, Vec<(Pat, Expr)>),
}

impl Expr {
    /// Zählt Knoten (für Split-Limit / Größen-Schranken).
    pub fn size(&self) -> usize {
        match self {
            Expr::Var(_) | Expr::Lit(_) => 1,
            Expr::BinOp(_, l, r) => 1 + l.size() + r.size(),
            Expr::Call(_, args) => 1 + args.iter().map(Expr::size).sum::<usize>(),
            Expr::Let(_, v, b) => 1 + v.size() + b.size(),
            Expr::Block(stmts, tail) => {
                1 + stmts.iter().map(Expr::size).sum::<usize>() + tail.size()
            }
            Expr::If(c, t, e) => 1 + c.size() + t.size() + e.size(),
            Expr::Match(s, arms) => {
                1 + s.size() + arms.iter().map(|(_, e)| e.size()).sum::<usize>()
            }
        }
    }

    /// Ob der Ausdruck "atomar" ist (Variable oder Literal).
    pub fn is_atom(&self) -> bool {
        matches!(self, Expr::Var(_) | Expr::Lit(_))
    }
}

/// Eine Funktionsdefinition der Test-IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunDef {
    pub name: String,
    /// Typparameter (leer = monomorph).
    pub type_params: Vec<String>,
    /// Konkrete Typen nach Mono, `None` = noch polymorph.
    pub type_args: Option<Vec<Ty>>,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub body: Expr,
}

impl FunDef {
    pub fn new(
        name: impl Into<String>,
        type_params: Vec<String>,
        params: Vec<(String, Ty)>,
        ret: Ty,
        body: Expr,
    ) -> Self {
        FunDef {
            name: name.into(),
            type_params,
            type_args: None,
            params,
            ret,
            body,
        }
    }

    pub fn is_polymorphic(&self) -> bool {
        !self.type_params.is_empty() && self.type_args.is_none()
    }

    pub fn size(&self) -> usize {
        self.body.size() + self.params.len()
    }
}
