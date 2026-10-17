#![forbid(unsafe_code)]
//! Printing context and the oracle that answers the environment queries the
//! OCaml printer makes against its typechecker state.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use sail_ast::{NExp, QuantItem, SimpleInterner, TypSchm};

use crate::doc::Doc;

/// Environment queries of `pretty_print_lean.ml` (`Env.*`, `Effects.*`,
/// `Type_check.prove`). Implemented over the frozen declaration tables.
pub trait Oracle {
    /// `Env.is_register`.
    fn is_register(&self, name: &str) -> bool;
    /// Enum member (`Type_check.is_enum_member` / `Env.lookup_id` = `Enum`).
    fn is_enum_member(&self, name: &str) -> bool;
    /// `Env.is_union_constructor`.
    fn is_union_ctor(&self, name: &str) -> bool;
    /// `Env.get_extern f env "lean"`.
    fn lean_extern(&self, name: &str) -> Option<String>;
    /// `Env.get_val_spec`: the declared scheme of a function.
    fn val_spec(&self, name: &str) -> Option<(TypSchm, Arc<SimpleInterner>)>;
    /// `Effects.function_is_pure`.
    fn function_is_pure(&self, name: &str) -> bool;
    /// `Bindings.find_opt f ctx.global.fun_args`.
    fn fun_args(&self, name: &str) -> Option<Vec<String>>;
    /// A value (function, register, enum member, constructor, global) is
    /// bound to this name, so a type of the same name must be qualified.
    fn value_named(&self, name: &str) -> bool;
    /// Name of the variant type whose constructor `ctor` clashes with a type
    /// of the same name (`compute_ctor_renames`).
    fn ctor_rename(&self, ctor: &str) -> Option<String>;
    /// Constant bit width of the type of a pattern node, if it has type
    /// `bits(n)`/`bitvector(n)` with constant `n` (`typ_of_pat`).
    fn pat_const_bits(&self, _pat: sail_ast::NodeId) -> Option<i128> {
        None
    }
    /// Whether the type of an expression node is a bitvector
    /// (`is_bitvector_typ (typ_of e)`), if known.
    fn exp_is_bitvector(&self, _exp: sail_ast::NodeId) -> Option<bool> {
        None
    }
    /// Whether the type of an expression node is `unit`, if known.
    fn exp_is_unit(&self, _exp: sail_ast::NodeId) -> Option<bool> {
        None
    }
    /// Head type constructor of `typ` after expanding type abbreviations.
    fn typ_head(&self, _typ: &sail_ast::Typ, _it: &SimpleInterner) -> Option<String> {
        None
    }
    /// `Type_check.prove (x >= 0)` under the given quantifier assumptions.
    fn provably_nneg(
        &self,
        quant: &[QuantItem],
        quant_it: &SimpleInterner,
        nexp: &NExp,
        nexp_it: &SimpleInterner,
    ) -> bool;
}

/// Global, mutable printing state shared by all definitions.
pub struct Global<'a> {
    pub oracle: &'a dyn Oracle,
    /// `ctx.global.out_name_camel`.
    pub out_name_camel: String,
    /// `kid_id_renames` established by global `let 'n = ...` bindings.
    pub kid_id_renames: HashMap<String, Option<String>>,
    pub kid_id_renames_rev: HashMap<String, String>,
    pub extern_types: Vec<String>,
    pub non_beq_types: Vec<String>,
    pub partial_functions: Vec<String>,
    pub noncomputable_functions: Vec<String>,
    pub the_main_function_has_been_seen: Cell<bool>,
    /// Types opened with `open` at the top of the generated files.
    pub opens: std::cell::RefCell<std::collections::BTreeSet<String>>,
}

impl<'a> Global<'a> {
    #[must_use]
    pub fn new(oracle: &'a dyn Oracle, out_name_camel: &str) -> Self {
        Global {
            oracle,
            out_name_camel: out_name_camel.to_owned(),
            kid_id_renames: HashMap::new(),
            kid_id_renames_rev: HashMap::new(),
            extern_types: Vec::new(),
            non_beq_types: Vec::new(),
            partial_functions: Vec::new(),
            noncomputable_functions: Vec::new(),
            the_main_function_has_been_seen: Cell::new(false),
            opens: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        }
    }

    /// `add_global_kid_id_rename`.
    pub fn add_kid_id_rename(&mut self, id: &str, kid: &str) {
        if let Some(previous) = self.kid_id_renames_rev.get(id).cloned() {
            self.kid_id_renames.insert(previous, None);
        }
        self.kid_id_renames.insert(kid.to_owned(), Some(id.to_owned()));
        self.kid_id_renames_rev.insert(id.to_owned(), kid.to_owned());
    }
}

/// The per-function printing context (`pretty_print_lean.ml: context`).
#[derive(Clone)]
pub struct Ctx<'a> {
    pub global: &'a Global<'a>,
    /// Definition interner: resolves every symbol of the printed AST.
    pub it: &'a SimpleInterner,
    /// Quantifier assumptions of the current function (for `provably_nneg`).
    pub quant: Option<(&'a [QuantItem], &'a SimpleInterner)>,
    pub kid_id_renames: HashMap<String, Option<String>>,
    pub kid_id_renames_rev: HashMap<String, String>,
    pub loop_level: Cell<u32>,
    pub in_sail_monad: bool,
    pub in_except_monad: Option<Doc>,
}

impl<'a> Ctx<'a> {
    #[must_use]
    pub fn new(global: &'a Global<'a>, it: &'a SimpleInterner) -> Self {
        Ctx {
            global,
            it,
            quant: None,
            kid_id_renames: global.kid_id_renames.clone(),
            kid_id_renames_rev: global.kid_id_renames_rev.clone(),
            loop_level: Cell::new(0),
            in_sail_monad: false,
            in_except_monad: None,
        }
    }

    /// `add_single_kid_id_rename`.
    pub fn add_kid_id_rename(&mut self, id: &str, kid: &str) {
        if let Some(previous) = self.kid_id_renames_rev.get(id).cloned() {
            self.kid_id_renames.insert(previous, None);
        }
        self.kid_id_renames.insert(kid.to_owned(), Some(id.to_owned()));
        self.kid_id_renames_rev.insert(id.to_owned(), kid.to_owned());
    }

    pub fn text(&self, symbol: sail_ast::Symbol) -> String {
        sail_ast::Interner::resolve(self.it, symbol)
            .unwrap_or("?")
            .to_owned()
    }
}
