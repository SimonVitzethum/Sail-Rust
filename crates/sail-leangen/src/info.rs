#![forbid(unsafe_code)]
//! Whole-program tables the printer queries (`populate_fun_args`,
//! `compute_ctor_renames`, extern spellings, effect summary).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use sail_ast::{
    Def, DefKind, Exp, ExpKind, LExp, LExpKind, PExpKind, PatKind, SimpleInterner, TypSchm, TypeDefKind,
};

use crate::analysis::exp_exists;
use crate::names::name_text;

/// One `val` specification.
#[derive(Clone)]
pub struct ValInfo {
    pub scheme: TypSchm,
    pub it: Arc<SimpleInterner>,
    /// `Env.get_extern f env "lean"`.
    pub lean_extern: Option<String>,
    /// `Env.is_extern`: any binding for lean (or `_`).
    pub is_extern: bool,
    /// `val f = impure {...}` / `pure {...}`.
    pub extern_pure: bool,
}

#[derive(Default)]
pub struct GlobalInfo {
    pub vals: HashMap<String, ValInfo>,
    /// Registers in declaration order with their types.
    pub registers: Vec<(String, sail_ast::Typ, Arc<SimpleInterner>)>,
    register_set: HashSet<String>,
    pub enum_members: HashSet<String>,
    /// Constructor -> variant type.
    pub union_ctors: HashMap<String, String>,
    pub type_names: HashSet<String>,
    pub fun_args: HashMap<String, Vec<String>>,
    pub defined_functions: HashSet<String>,
    pub global_lets: HashSet<String>,
    direct_effect: HashMap<String, bool>,
    overloads: HashMap<String, Vec<String>>,
    callees: HashMap<String, Vec<String>>,
    impure: HashSet<String>,
    /// Union constructors named like a type.
    ctor_renames: HashMap<String, String>,
}

fn lean_binding(bindings: &[(String, String)]) -> Option<String> {
    bindings
        .iter()
        .find(|(k, _)| k == "lean")
        .or_else(|| bindings.iter().find(|(k, _)| k == "_"))
        .map(|(_, v)| v.clone())
}

impl GlobalInfo {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_register(&self, name: &str) -> bool {
        self.register_set.contains(name)
    }

    /// Record one (rewritten) definition.
    pub fn observe(&mut self, def: &Def, it: &Arc<SimpleInterner>) {
        match &def.kind {
            DefKind::Val(vs) => {
                let name = name_text(&vs.name, it);
                let (lean_extern, is_extern, extern_pure) = match &vs.extern_opt {
                    Some(ext) => {
                        let lean = lean_binding(&ext.bindings);
                        (lean.clone(), lean.is_some(), ext.pure)
                    }
                    None => (None, false, true),
                };
                self.vals.insert(
                    name,
                    ValInfo {
                        scheme: vs.typschm.clone(),
                        it: it.clone(),
                        lean_extern,
                        is_extern,
                        extern_pure,
                    },
                );
            }
            DefKind::Register(spec) => {
                let name = name_text(&spec.name, it);
                self.register_set.insert(name.clone());
                self.registers.push((name, spec.typ.clone(), it.clone()));
            }
            DefKind::Type(td) => match &td.kind {
                TypeDefKind::Enum { name, members, .. } => {
                    self.type_names.insert(name_text(name, it));
                    for m in members {
                        self.enum_members.insert(name_text(m, it));
                    }
                }
                TypeDefKind::Variant { name, ctors, .. } => {
                    let typ = name_text(name, it);
                    self.type_names.insert(typ.clone());
                    for c in ctors {
                        self.union_ctors.insert(name_text(&c.ctor, it), typ.clone());
                    }
                }
                TypeDefKind::Record { name, .. }
                | TypeDefKind::Abbrev { name, .. }
                | TypeDefKind::Abstract { name, .. }
                | TypeDefKind::Bitfield { name, .. } => {
                    self.type_names.insert(name_text(name, it));
                }
            },
            DefKind::Fundef { funcls, .. } => {
                if let Some(first) = funcls.first() {
                    let name = name_text(&first.name, it);
                    self.defined_functions.insert(name.clone());
                    if funcls.len() == 1 {
                        if let Some(args) = fun_arg_names(first, it) {
                            self.fun_args.insert(name.clone(), args);
                        }
                    }
                    let mut direct = false;
                    let mut callees = Vec::new();
                    for f in funcls {
                        match &f.arm.kind {
                            PExpKind::PatExp { body, .. } => scan_exp(body, it, self, &mut direct, &mut callees),
                            PExpKind::PatWhen { guard, body, .. } => {
                                scan_exp(guard, it, self, &mut direct, &mut callees);
                                scan_exp(body, it, self, &mut direct, &mut callees);
                            }
                        }
                    }
                    *self.direct_effect.entry(name.clone()).or_default() |= direct;
                    self.callees.entry(name).or_default().extend(callees);
                }
            }
            DefKind::InternalMutrec(sets) => {
                for set in sets {
                    if let Some(first) = set.funcls.first() {
                        let name = name_text(&first.name, it);
                        self.defined_functions.insert(name.clone());
                        if set.funcls.len() == 1 {
                            if let Some(args) = fun_arg_names(first, it) {
                                self.fun_args.insert(name.clone(), args);
                            }
                        }
                        let mut direct = false;
                        let mut callees = Vec::new();
                        for f in &set.funcls {
                            match &f.arm.kind {
                                PExpKind::PatExp { body, .. } => scan_exp(body, it, self, &mut direct, &mut callees),
                                PExpKind::PatWhen { guard, body, .. } => {
                                    scan_exp(guard, it, self, &mut direct, &mut callees);
                                    scan_exp(body, it, self, &mut direct, &mut callees);
                                }
                            }
                        }
                        *self.direct_effect.entry(name.clone()).or_default() |= direct;
                        self.callees.entry(name).or_default().extend(callees);
                    }
                }
            }
            DefKind::Let { pat, exp } => {
                let mut ids = HashSet::new();
                collect_pat_ids(pat, it, &mut ids);
                let mut direct = false;
                let mut callees = Vec::new();
                scan_exp(exp, it, self, &mut direct, &mut callees);
                for id in &ids {
                    *self.direct_effect.entry(id.clone()).or_default() |= direct;
                    self.callees.entry(id.clone()).or_default().extend(callees.iter().cloned());
                }
                self.global_lets.extend(ids);
            }
            DefKind::Overload { name, overloads } => {
                let members = overloads.iter().map(|m| name_text(m, it)).collect();
                self.overloads.insert(name_text(name, it), members);
            }
            _ => {}
        }
    }

    /// Fix point of `Effects.infer_side_effects`: a function is impure when
    /// it has a direct effect, is an impure extern, or calls an impure function.
    pub fn finish(&mut self) {
        // Type names shadowed by constructors (`compute_ctor_renames`).
        for (ctor, typ) in &self.union_ctors {
            if self.type_names.contains(ctor) {
                self.ctor_renames.insert(ctor.clone(), typ.clone());
            }
        }
        let mut impure: HashSet<String> = HashSet::new();
        for (name, info) in &self.vals {
            if info.is_extern_impure() {
                impure.insert(name.clone());
            }
        }
        for (name, direct) in &self.direct_effect {
            if *direct {
                impure.insert(name.clone());
            }
        }
        // Reverse edges for propagation; an overloaded callee stands for all
        // of its members.
        let mut callers: HashMap<String, Vec<String>> = HashMap::new();
        for (f, cs) in &self.callees {
            for c in cs {
                let mut stack = vec![c.clone()];
                let mut seen: HashSet<String> = HashSet::new();
                while let Some(name) = stack.pop() {
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    if let Some(members) = self.overloads.get(&name) {
                        stack.extend(members.iter().cloned());
                    }
                    callers.entry(name).or_default().push(f.clone());
                }
            }
        }
        let mut work: Vec<String> = impure.iter().cloned().collect();
        while let Some(f) = work.pop() {
            if let Some(cs) = callers.get(&f) {
                for caller in cs {
                    if impure.insert(caller.clone()) {
                        work.push(caller.clone());
                    }
                }
            }
        }
        self.impure = impure;
    }

    pub fn function_is_pure(&self, name: &str) -> bool {
        !self.impure.contains(name)
    }

    pub fn ctor_rename(&self, ctor: &str) -> Option<&String> {
        self.ctor_renames.get(ctor)
    }

    /// The registers' distinct types in first-use order (`add_reg_typ`).
    #[must_use]
    pub fn sorted_vals(&self) -> BTreeMap<&String, &ValInfo> {
        self.vals.iter().collect()
    }
}

impl ValInfo {
    /// An extern that is not marked pure (`impure` or unmarked).
    fn is_extern_impure(&self) -> bool {
        !self.extern_pure
    }
}

fn collect_pat_ids(pat: &sail_ast::Pat, it: &SimpleInterner, out: &mut HashSet<String>) {
    match &pat.kind {
        PatKind::Id(id) => {
            out.insert(name_text(id, it));
        }
        PatKind::Typ { pat, .. } | PatKind::Var { pat, .. } => collect_pat_ids(pat, it, out),
        PatKind::As { pat, id } => {
            out.insert(name_text(id, it));
            collect_pat_ids(pat, it, out);
        }
        PatKind::Tuple(ps) => {
            for p in ps {
                collect_pat_ids(p, it, out);
            }
        }
        _ => {}
    }
}

/// `populate_fun_args` for one single-clause function.
fn fun_arg_names(funcl: &sail_ast::Funcl, it: &SimpleInterner) -> Option<Vec<String>> {
    let pat = match &funcl.arm.kind {
        PExpKind::PatExp { pat, .. } => pat,
        PExpKind::PatWhen { .. } => return None,
    };
    let id_of = |p: &sail_ast::Pat| -> String {
        match &p.kind {
            PatKind::Id(id) => name_text(id, it),
            PatKind::Typ { pat, .. } => match &pat.kind {
                PatKind::Id(id) => name_text(id, it),
                _ => String::new(),
            },
            _ => String::new(),
        }
    };
    match &pat.kind {
        PatKind::Tuple(ps) => Some(ps.iter().map(id_of).collect()),
        PatKind::Id(id) => Some(vec![name_text(id, it)]),
        PatKind::Typ { pat, .. } => match &pat.kind {
            PatKind::Id(id) => Some(vec![name_text(id, it)]),
            _ => None,
        },
        _ => None,
    }
}

fn lexp_direct(l: &LExp, it: &SimpleInterner, info: &GlobalInfo) -> bool {
    match &l.kind {
        LExpKind::Id(id) | LExpKind::Typ { id, .. } => info.is_register(&name_text(id, it)),
        LExpKind::Deref(_) => true,
        LExpKind::Tuple(items) | LExpKind::VectorConcat(items) => items.iter().any(|x| lexp_direct(x, it, info)),
        LExpKind::Vector { lexp, .. } | LExpKind::VectorRange { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_direct(lexp, it, info)
        }
    }
}

fn scan_exp(e: &Exp, it: &SimpleInterner, info: &GlobalInfo, direct: &mut bool, callees: &mut Vec<String>) {
    exp_exists(e, &mut |x| {
        match &x.kind {
            ExpKind::Id(id) => {
                let text = name_text(id, it);
                if info.is_register(&text) {
                    *direct = true;
                } else if info.global_lets.contains(&text) {
                    callees.push(text);
                }
            }
            ExpKind::Undef | ExpKind::Throw(_) | ExpKind::Exit(_) | ExpKind::Assert { .. } => *direct = true,
            ExpKind::App { func, .. } => {
                let name = name_text(func, it);
                if name == "__deref" {
                    *direct = true;
                } else {
                    callees.push(name);
                }
            }
            ExpKind::Assign { lexp, .. } | ExpKind::Var { lexp, .. } => {
                if lexp_direct(lexp, it, info) {
                    *direct = true;
                }
            }
            ExpKind::Match { .. } => {
                if x.annotation
                    .as_ref()
                    .is_some_and(|a| a.attributes.iter().any(|attr| attr.name == "incomplete"))
                {
                    *direct = true;
                }
            }
            _ => {}
        }
        false
    });
}
