//! Dense-index callgraph over [`crate::ir::FunDef`].
//!
//! Eigenbau statt petgraph: `HashMap<String, u32>` + `Vec<Vec<u32>>`
//! für Kanten und Rückkanten. Keine `unsafe`, keine externen Deps,
//! klone-arm (Namen werden einmal interniert).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use crate::ir::{Block, Expr, FunDef, Stmt};

/// Dense node id (index into the graph's tables).
pub type NodeId = u32;

/// Callgraph with forward and reverse adjacency.
#[derive(Clone, Debug, Default)]
pub struct CallGraph {
    /// Interned function names: id -> name.
    names: Vec<String>,
    /// name -> id.
    index: HashMap<String, NodeId>,
    /// Callees per function (deduped, sorted for determinism).
    callees: Vec<Vec<NodeId>>,
    /// Callers per function (deduped, sorted for determinism).
    callers: Vec<Vec<NodeId>>,
}

impl CallGraph {
    /// Build the graph from a program. Pure function of the input
    /// slice; unknown callees (externs/primitives) become dangling
    /// ids with no outgoing edges so transitive queries still work.
    pub fn build(program: &[FunDef]) -> Self {
        let mut graph = Self::default();
        // Intern all defined functions first (stable order = input order).
        for def in program {
            graph.intern(&def.name);
        }
        // Intern callees (externs) on demand, then record edges.
        let mut edges: Vec<(NodeId, NodeId)> = Vec::new();
        for def in program {
            let caller = graph.index[def.name.as_str()];
            let mut seen = Vec::new();
            collect_calls(&def.body, &mut seen);
            for callee_name in seen {
                let callee = graph.intern(&callee_name);
                edges.push((caller, callee));
            }
        }
        // Size adjacency for extern nodes added during edge collection.
        graph.callees.resize(graph.names.len(), Vec::new());
        graph.callers.resize(graph.names.len(), Vec::new());
        for (caller, callee) in edges {
            graph.callees[caller as usize].push(callee);
            graph.callers[callee as usize].push(caller);
        }
        // Dedup + sort for deterministic output.
        for adj in graph.callees.iter_mut().chain(graph.callers.iter_mut()) {
            adj.sort_unstable();
            adj.dedup();
        }
        graph
    }

    /// Number of nodes (defined functions + referenced externs).
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Id of a function by name, if present.
    pub fn id_of(&self, name: &str) -> Option<NodeId> {
        self.index.get(name).copied()
    }

    /// Name of a node id, if valid.
    pub fn name_of(&self, id: NodeId) -> Option<&str> {
        self.names.get(id as usize).map(String::as_str)
    }

    /// All function names in id order.
    pub fn all_functions(&self) -> &[String] {
        &self.names
    }

    /// Direct callees of `name` (empty if unknown).
    pub fn callees_of(&self, name: &str) -> Vec<&str> {
        self.id_of(name)
            .map(|id| self.callees[id as usize].iter().map(|&c| self.names[c as usize].as_str()).collect())
            .unwrap_or_default()
    }

    /// Direct callers of `name` (empty if unknown).
    pub fn callers_of(&self, name: &str) -> Vec<&str> {
        self.id_of(name)
            .map(|id| self.callers[id as usize].iter().map(|&c| self.names[c as usize].as_str()).collect())
            .unwrap_or_default()
    }

    /// True if `caller` directly calls `callee`.
    pub fn has_edge(&self, caller: &str, callee: &str) -> bool {
        match (self.id_of(caller), self.id_of(callee)) {
            (Some(a), Some(b)) => self.callees[a as usize].contains(&b),
            _ => false,
        }
    }

    /// All functions reachable from `name` (excluding `name` itself),
    /// in DFS order. Pure traversal over immutable borrows.
    pub fn reachable_from(&self, name: &str) -> Vec<&str> {
        let Some(start) = self.id_of(name) else {
            return Vec::new();
        };
        let mut visited = vec![false; self.names.len()];
        visited[start as usize] = true;
        let mut stack = vec![start];
        let mut order = Vec::new();
        while let Some(id) = stack.pop() {
            for &callee in &self.callees[id as usize] {
                if !visited[callee as usize] {
                    visited[callee as usize] = true;
                    order.push(self.names[callee as usize].as_str());
                    stack.push(callee);
                }
            }
        }
        order
    }

    fn intern(&mut self, name: &str) -> NodeId {
        if let Some(&id) = self.index.get(name) {
            return id;
        }
        let id = self.names.len() as NodeId;
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), id);
        self.callees.push(Vec::new());
        self.callers.push(Vec::new());
        id
    }
}

fn collect_calls(block: &Block, out: &mut Vec<String>) {
    for stmt in &block.stmts {
        collect_stmt_calls(stmt, out);
    }
}

fn collect_stmt_calls(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Let { expr, .. } | Stmt::Assign { expr, .. } | Stmt::Return(expr) | Stmt::Throw(expr) | Stmt::Assert(expr) | Stmt::ExprStmt(expr) => {
            collect_expr_calls(expr, out);
        }
        Stmt::WriteReg { expr, .. } => collect_expr_calls(expr, out),
        Stmt::WriteMem { addr, value } => {
            collect_expr_calls(addr, out);
            collect_expr_calls(value, out);
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_expr_calls(cond, out);
            collect_calls(then_branch, out);
            collect_calls(else_branch, out);
        }
        Stmt::While { cond, body } => {
            collect_expr_calls(cond, out);
            collect_calls(body, out);
        }
        Stmt::For { lo, hi, body, .. } => {
            collect_expr_calls(lo, out);
            collect_expr_calls(hi, out);
            collect_calls(body, out);
        }
    }
}

fn collect_expr_calls(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Call { func, args } => {
            if !out.iter().any(|f| f == func) {
                out.push(func.clone());
            }
            for arg in args {
                collect_expr_calls(arg, out);
            }
        }
        Expr::Binop { lhs, rhs, .. } => {
            collect_expr_calls(lhs, out);
            collect_expr_calls(rhs, out);
        }
        Expr::ReadMem(addr) => collect_expr_calls(addr, out),
        Expr::Lit(_) | Expr::Var(_) | Expr::ReadReg(_) | Expr::Nondet => {}
    }
}
