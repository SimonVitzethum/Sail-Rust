#![forbid(unsafe_code)]
//! Register model: `RegisterDef` extraction plus `doc_reg_info` equivalent.
//!
//! Reference: `sail/src/sail_lean_backend/pretty_print_lean.ml`
//! (`register_enums`, `type_enum`, `doc_reg_info`, `doc_instantiations`,
//! `readReg`/`writeReg` at the `E_id`/`E_assign` cases) and
//! `sail/src/lib/state.ml` (`find_registers`, `generate_regstate`,
//! `id_of_regtyp`, grouped-regstate flag, initial-value defaults).
//!
//! Lane-01 mapping: [`MinimalDef::Register`] mirrors
//! `sail-ast` `DefKind::Register(DecSpec)` (`typ`, `name`,
//! `init: Option<Exp>`); see `REGS_NEEDED.md`. The [`bridge`] module converts
//! real `sail-ast` defs (migration entries #26/#27); the local minimal types
//! below stay as the plain-data input surface.
//!
//! Lane 61 status (MIGRATION.md #26/#27): [`MinimalDef`]/[`SailTyp`] are
//! retained — renderings (`bits(64)` spellings, optional init text) need
//! the Lane-10 `TypKind` pretty-printer and a real expression grammar
//! before they can lower onto `DecSpec{typ: Typ, init: Option<Exp>}`,
//! and `sail-cli` consumes this input API. The crate depends on
//! `sail-ast` so the swap seam is wired.

use sail_diag::{DiagCode, SailError, Span};
use std::collections::BTreeMap;
use std::fmt::Write;

pub mod bridge;

pub use bridge::{
    dec_spec_to_minimal, extract_registers_ast, lean_nexp_of_ast, lean_typ_of_ast,
    lean_typ_string_of_ast, render_sail_constraint, render_sail_lit, render_sail_nexp,
    render_sail_typ, render_sail_typarg, try_render_sail_exp,
};

/// A Sail type as written in source, e.g. `bits(64)`, `bool`, `int`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SailTyp(pub String);

impl SailTyp {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        SailTyp(s.into())
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Minimal top-level definition input.
///
/// `Register` mirrors lane-01 `DefKind::Register(DecSpec)`:
/// `name` = `DecSpec.name`, `typ` = `DecSpec.typ` (rendered),
/// `init` = `DecSpec.init` rendered to Sail source when present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MinimalDef {
    Register {
        name: String,
        typ: SailTyp,
        init: Option<String>,
    },
    Other,
}

/// One extracted register: `(typ, id, has_init)` triple of
/// `State.find_registers` plus the rendered pieces this crate needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterDef {
    /// Sail register name, e.g. `PC`.
    pub name: String,
    /// Sail type as written, e.g. `bits(64)`.
    pub sail_typ: String,
    /// Lean type (`doc_typ` equivalent), e.g. `(BitVec 64)`.
    pub lean_typ: String,
    /// `true` when the Sail def carries an initializer (`= ...`).
    pub has_init: bool,
    /// Rendered Sail initializer when `has_init`.
    pub init_sail: Option<String>,
}

impl RegisterDef {
    /// Lean constructor suffix for `.Name` (`doc_id_ctor` equivalent;
    /// register names are already `CamelCase`/upper, kept verbatim).
    #[must_use]
    pub fn ctor(&self) -> &str {
        &self.name
    }
}

/// Extract registers from minimal defs (`find_registers` equivalent).
///
/// Order is definition order (OCaml accumulates reversed then the
/// backends consume the list as returned; definition order is the
/// stable, documented choice here).
#[must_use]
pub fn extract_registers(defs: &[MinimalDef]) -> Vec<RegisterDef> {
    defs.iter()
        .filter_map(|d| match d {
            MinimalDef::Register { name, typ, init } => Some(RegisterDef {
                name: name.clone(),
                sail_typ: typ.as_str().to_owned(),
                lean_typ: lean_of_sail_typ(typ.as_str()),
                has_init: init.is_some(),
                init_sail: init.clone(),
            }),
            MinimalDef::Other => None,
        })
        .collect()
}

/// Parse `register <name> : <typ> [= <init>]` lines out of Sail source.
///
/// Tiny line-based helper for fixtures/tests; the real frontend feeds
/// [`MinimalDef`] directly (see `REGS_NEEDED.md`). Lines starting with
/// `//` and empty lines are skipped; trailing `//` comments are cut.
#[must_use]
pub fn parse_register_lines(src: &str) -> Vec<MinimalDef> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        }
        .trim()
        .trim_end_matches(';')
        .trim();
        if line.is_empty() {
            continue;
        }
        let rest = match line.strip_prefix("register") {
            Some(r) if r.starts_with(char::is_whitespace) => r.trim(),
            _ => continue,
        };
        let (name, rest) = match rest.find(':') {
            Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
            None => continue,
        };
        if name.is_empty() || rest.is_empty() {
            continue;
        }
        let (typ, init) = match rest.find('=') {
            Some(i) => {
                let t = rest[..i].trim().to_owned();
                let v = rest[i + 1..].trim().to_owned();
                (t, if v.is_empty() { None } else { Some(v) })
            }
            None => (rest.to_owned(), None),
        };
        if typ.is_empty() {
            continue;
        }
        out.push(MinimalDef::Register {
            name: name.to_owned(),
            typ: SailTyp::new(typ),
            init,
        });
    }
    out
}

/// Map a Sail type as written to its Lean rendering (`doc_typ` subset).
///
/// Handles `bits(n)`/`bitvector(n)` → `(BitVec n)`, `bit` → `(BitVec 1)`,
/// `bool` → `Bool`, `int` → `Int`, `nat` → `Nat`, `string` → `String`,
/// `unit` → `Unit`, `vector(n, t)` → `(Vector <t> n)`, `option(t)` →
/// `(Option <t>)`, `list(t)` → `(List <t>)`; anything else passes through
/// verbatim (user type constructors render via `doc_id_typ_ctor` anyway).
#[must_use]
pub fn lean_of_sail_typ(sail: &str) -> String {
    let s: String = sail.chars().filter(|c| !c.is_whitespace()).collect();
    if s == "bool" {
        return "Bool".to_owned();
    }
    if s == "int" {
        return "Int".to_owned();
    }
    if s == "nat" {
        return "Nat".to_owned();
    }
    if s == "string" {
        return "String".to_owned();
    }
    if s == "unit" {
        return "Unit".to_owned();
    }
    if s == "bit" {
        return "(BitVec 1)".to_owned();
    }
    for prefix in ["bits(", "bitvector("] {
        if let Some(inner) = s.strip_prefix(prefix).and_then(|r| r.strip_suffix(')')) {
            return format!("(BitVec {inner})");
        }
    }
    if let Some(inner) = s.strip_prefix("vector(").and_then(|r| r.strip_suffix(')')) {
        if let Some((n, t)) = split_top_level_comma(inner) {
            return format!("(Vector {} {n})", lean_of_sail_typ(t));
        }
    }
    if let Some(inner) = s.strip_prefix("option(").and_then(|r| r.strip_suffix(')')) {
        return format!("(Option {})", lean_of_sail_typ(inner));
    }
    if let Some(inner) = s.strip_prefix("list(").and_then(|r| r.strip_suffix(')')) {
        return format!("(List {})", lean_of_sail_typ(inner));
    }
    sail.trim().to_owned()
}

fn split_top_level_comma(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some((s[..i].trim(), s[i + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// `register ...` identifier used for grouped-regstate fields
/// (`State.id_of_regtyp`/`regstate_field` equivalent, simplified):
/// alphanumerics kept, anything else → `_`.
#[must_use]
pub fn regtyp_field_stem(sail_typ: &str) -> String {
    let mut stem: String = sail_typ
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    while stem.contains("__") {
        stem = stem.replace("__", "_");
    }
    stem.trim_matches('_').to_owned()
}

/// Whole-register IR: `doc_reg_info` equivalent.
///
/// Holds the `(typ, id, has_init)` list plus every Lean snippet the
/// backend needs: `Register` inductive, `RegisterType` abbrev,
/// `Inhabited (RegisterRef …)` instances, `readReg`/`writeReg`
/// expression mapping, grouped/ungrouped `regstate`, and the
/// `reset.sail` initial-value interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterInfo {
    pub registers: Vec<RegisterDef>,
}

impl RegisterInfo {
    #[must_use]
    pub fn new(registers: Vec<RegisterDef>) -> Self {
        RegisterInfo { registers }
    }

    #[must_use]
    pub fn empty() -> Self {
        RegisterInfo {
            registers: Vec::new(),
        }
    }

    #[must_use]
    pub fn has_registers(&self) -> bool {
        !self.registers.is_empty()
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&RegisterDef> {
        self.registers.iter().find(|r| r.name == name)
    }

    /// `register_enums` equivalent: `inductive Register …`.
    #[must_use]
    pub fn register_enum(&self) -> String {
        let mut s = String::from("inductive Register : Type where\n");
        for r in &self.registers {
            let _ = writeln!(s, "  | {}", r.ctor());
        }
        s.push_str("  deriving DecidableEq, Hashable, Repr\nopen Register\n");
        s
    }

    /// `type_enum` equivalent: `abbrev RegisterType : Register → Type`.
    #[must_use]
    pub fn register_type_abbrev(&self) -> String {
        let mut s = String::from("abbrev RegisterType : Register → Type\n");
        for r in &self.registers {
            let _ = writeln!(s, "  | .{} => {}", r.ctor(), r.lean_typ);
        }
        s
    }

    /// `inhabit_enum_v2` equivalent (current concurrency interface).
    ///
    /// One instance per distinct Lean type, keeping the first constructor
    /// (`pretty_print_lean.ml`: `add_reg_typ` folds registers into a
    /// `typ_map` keyed by type, so registers sharing a type — e.g. `PC`
    /// and `SP`, both `(BitVec 64)` — yield a single instance; same rule
    /// as `sail-lake::render_register_inhabited`).
    #[must_use]
    pub fn inhabited_instances(&self) -> String {
        let mut s = String::new();
        let mut seen: Vec<&str> = Vec::new();
        for r in &self.registers {
            if seen.contains(&r.lean_typ.as_str()) {
                continue;
            }
            seen.push(r.lean_typ.as_str());
            let _ = writeln!(
                s,
                "instance : Inhabited (RegisterRef {}) where\n  default := .Reg {}",
                r.lean_typ,
                r.ctor()
            );
        }
        s
    }

    /// `doc_reg_info` equivalent: enum + type abbrev (+ trailing blank).
    ///
    /// Exactly `register_enums` + `type_enum`
    /// (`pretty_print_lean.ml:1499-1501`); the `Inhabited` instances are
    /// **not** part of `doc_reg_info` — the reference backend emits them
    /// from `doc_instantiations` (`inhabit_enum_v1/v2`). Use
    /// [`Self::doc_instantiations`] for that fragment.
    /// Empty register files print the `PEmpty` fallback instead.
    #[must_use]
    pub fn doc_reg_info(&self) -> String {
        if self.registers.is_empty() {
            return "abbrev Register := PEmpty\nabbrev RegisterType : Register -> Type := PEmpty.elim\n\n"
                .to_owned();
        }
        format!("{}\n{}\n", self.register_enum(), self.register_type_abbrev(),)
    }

    /// `doc_instantiations` fragment owned by the register model: the
    /// deduped `Inhabited (RegisterRef …)` instances (`inhabit_enum_v2`).
    ///
    /// The `Arch` instance and the `SailM`/`SailME` monad abbrevs of
    /// `doc_instantiations_v1/v2` live in `sail-lake` (golden contract);
    /// this is only the per-type inhabited block. Empty for register-free
    /// models.
    #[must_use]
    pub fn doc_instantiations(&self) -> String {
        self.inhabited_instances()
    }

    /// Lean type of a register value reference: `(RegisterRef T)`
    /// (the `register(t)` case of `doc_typ`).
    #[must_use]
    pub fn register_ref_ty(inner_lean_typ: &str) -> String {
        format!("(RegisterRef {inner_lean_typ})")
    }

    /// `E_id` register case in monadic position: `readReg <Ctor>`
    /// (`pretty_print_lean.ml:859`). Returns `None` for non-registers so
    /// callers can fall back to plain variable printing. For pure
    /// (`as_monadic == false`) positions use [`Self::read_reg_in`], which
    /// applies the `←` arrow the reference backend wraps around the read.
    #[must_use]
    pub fn read_reg(&self, name: &str) -> Option<String> {
        self.read_reg_in(name, true)
    }

    /// `E_id` register case with arrow discipline (`pretty_print_lean.ml:859`
    /// via `wrap_with_left_arrow (not as_monadic)`): monadic positions
    /// print `readReg <Ctor>`, pure positions `(← readReg <Ctor>)`.
    #[must_use]
    pub fn read_reg_in(&self, name: &str, as_monadic: bool) -> Option<String> {
        self.lookup(name).map(|r| {
            if as_monadic {
                format!("readReg {}", r.ctor())
            } else {
                format!("(← readReg {})", r.ctor())
            }
        })
    }

    /// `E_assign` register case in monadic position: `writeReg <Ctor> <val>`
    /// (`pretty_print_lean.ml:1046`). Returns `None` for non-registers.
    /// For pure positions use [`Self::write_reg_in`].
    #[must_use]
    pub fn write_reg(&self, name: &str, val_lean: &str) -> Option<String> {
        self.write_reg_in(name, val_lean, true)
    }

    /// `E_assign` register case with arrow discipline
    /// (`pretty_print_lean.ml:1044-1046` via
    /// `wrap_with_left_arrow (not as_monadic)`): monadic positions print
    /// `writeReg <Ctor> <val>`, pure positions `(← writeReg <Ctor> <val>)`.
    #[must_use]
    pub fn write_reg_in(&self, name: &str, val_lean: &str, as_monadic: bool) -> Option<String> {
        self.lookup(name).map(|r| {
            if as_monadic {
                format!("writeReg {} {val_lean}", r.ctor())
            } else {
                format!("(← writeReg {} {val_lean})", r.ctor())
            }
        })
    }

    /// `E_assign` through a dynamic ref: `writeRegRef <ref> <val>`
    /// (pretty_print_lean.ml:1047).
    #[must_use]
    pub fn write_reg_ref(ref_lean: &str, val_lean: &str) -> String {
        format!("writeRegRef {ref_lean} {val_lean}")
    }

    /// `generate_regstate` equivalent.
    ///
    /// Ungrouped (default): one record field per register.
    /// Grouped (`-grouped_regstate`): one `<stem>_reg` field per
    /// distinct type stem holding a `string → T` function
    /// (`regstate_field`/`regval_base_typ` equivalent).
    #[must_use]
    pub fn regstate(&self, grouped: bool) -> String {
        if self.registers.is_empty() {
            return "abbrev regstate := Unit\n".to_owned();
        }
        if !grouped {
            let mut s = String::from("structure regstate where\n");
            for r in &self.registers {
                let _ = writeln!(s, "  {} : {}", r.name, r.lean_typ);
            }
            return s;
        }
        let mut fields: BTreeMap<String, &RegisterDef> = BTreeMap::new();
        for r in &self.registers {
            fields
                .entry(format!("{}_reg", regtyp_field_stem(&r.sail_typ)))
                .or_insert(r);
        }
        let mut s = String::from("structure regstate where\n");
        for (field, r) in &fields {
            let _ = writeln!(s, "  {field} : String → {}", r.lean_typ);
        }
        s
    }

    /// `reset.sail` initial-value interface (`generate_initial_regstate`
    /// equivalent). One entry per register: the Sail initializer when
    /// present, else the type default (`false`/`0`/`0b…`/`()` mirroring
    /// `lookup_init_val` in `state.ml`).
    #[must_use]
    pub fn initial_values(&self) -> Vec<InitialValue> {
        self.registers
            .iter()
            .map(|r| {
                let (value_sail, from_reset) = match &r.init_sail {
                    Some(v) => (v.clone(), true),
                    None => (default_init_sail(&r.sail_typ), false),
                };
                InitialValue {
                    register: r.name.clone(),
                    value_sail,
                    from_reset,
                }
            })
            .collect()
    }

    /// Lean stub wiring the initial values into a `regstate`
    /// (`initial_regstate` hook the backend emits next to the
    /// `reset.sail` functions).
    ///
    /// E2 (`DECISIONS.md`): this crate never emits a `sorry` placeholder.
    /// Translating the Sail initializer expressions in
    /// [`Self::initial_values`] to Lean terms needs the Sail→Lean
    /// expression printer, which is not wired up here — so a non-empty
    /// register file is a hard [`SailError`]
    /// ([`DiagCode::ExpressionNotTranslatable`], carrying the caller's
    /// `span`) instead of a silent stub. Pass the definition site of the
    /// register block (or [`Span::dummy`] for synthesized locations);
    /// the error hint points at the owning lane. The empty register file
    /// is exactly translatable (`regstate` is `Unit`), so it returns
    /// `def initial_regstate : regstate := ()`.
    #[must_use]
    pub fn initial_regstate_snippet(
        &self,
        grouped: bool,
        span: Span,
    ) -> Result<String, SailError> {
        if self.registers.is_empty() {
            return Ok("def initial_regstate : regstate := ()\n".to_owned());
        }
        let first = &self.initial_values()[0];
        Err(SailError::from_entry(
            span,
            DiagCode::ExpressionNotTranslatable,
            format!(
                "cannot emit `initial_regstate{}` for {} register(s): Sail initializer `{}` for `{}` is not translated to Lean; translate the `reset.sail` initializers to Lean terms first instead of emitting a placeholder",
                if grouped { "_grouped" } else { "" },
                self.registers.len(),
                first.value_sail,
                first.register,
            ),
        ))
    }
}

/// One register's initial value for the `reset.sail` interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialValue {
    /// Register name.
    pub register: String,
    /// Sail expression for the initial value (explicit `reset.sail`
    /// initializer or type default).
    pub value_sail: String,
    /// `true` when the value comes from an explicit `reset.sail`
    /// initializer rather than the type default.
    pub from_reset: bool,
}

/// Type defaults mirroring `lookup_init_val` (`state.ml`):
/// `bool`→`false`, `int`/`nat`→`0`, `string`→`""`, `unit`→`()`,
/// `bits(n)`/`bitvector(n)`→`0b0…0`, `bit`→`0b0`, else `undefined_<stem>`.
#[must_use]
pub fn default_init_sail(sail_typ: &str) -> String {
    let s: String = sail_typ.chars().filter(|c| !c.is_whitespace()).collect();
    match s.as_str() {
        "bool" => "false".to_owned(),
        "int" | "nat" => "0".to_owned(),
        "string" => "\"\"".to_owned(),
        "unit" => "()".to_owned(),
        "bit" => "0b0".to_owned(),
        _ => {
            for prefix in ["bits(", "bitvector("] {
                if let Some(inner) = s.strip_prefix(prefix).and_then(|r| r.strip_suffix(')')) {
                    if let Ok(n) = inner.parse::<usize>() {
                        return format!("0b{}", "0".repeat(n));
                    }
                    return format!("0b0 -- {sail_typ}");
                }
            }
            format!("undefined_{}", regtyp_field_stem(sail_typ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = "\
register PC : bits(64) = 0b0000000000000000000000000000000000000000000000000000000000000000\n\
register SP : bits(64)\n\
register PSTATE : bits(32) = 0b00000000000000000000000000000000\n";

    fn small_info() -> RegisterInfo {
        RegisterInfo::new(extract_registers(&parse_register_lines(SMALL)))
    }

    #[test]
    fn extracts_three_registers_in_order() {
        let regs = extract_registers(&parse_register_lines(SMALL));
        assert_eq!(
            regs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["PC", "SP", "PSTATE"]
        );
        assert_eq!(regs[0].lean_typ, "(BitVec 64)");
        assert_eq!(regs[2].lean_typ, "(BitVec 32)");
        assert!(regs[0].has_init);
        assert!(!regs[1].has_init);
        assert!(regs[2].has_init);
    }

    #[test]
    fn doc_reg_info_holds_enum_and_abbrev_only() {
        // `doc_reg_info` is exactly `register_enums` + `type_enum`
        // (pretty_print_lean.ml:1499-1501): no `Inhabited` instances here,
        // those come from `doc_instantiations`.
        let out = small_info().doc_reg_info();
        assert!(out.contains("inductive Register : Type where"), "{out}");
        assert!(out.contains("  | PC\n"), "{out}");
        assert!(out.contains("  | SP\n"), "{out}");
        assert!(out.contains("  | PSTATE\n"), "{out}");
        assert!(out.contains("deriving DecidableEq, Hashable, Repr"), "{out}");
        assert!(out.contains("abbrev RegisterType : Register → Type"), "{out}");
        assert!(out.contains("  | .PC => (BitVec 64)"), "{out}");
        assert!(out.contains("  | .PSTATE => (BitVec 32)"), "{out}");
        assert!(
            !out.contains("Inhabited"),
            "inhabited instances belong to doc_instantiations, not doc_reg_info:\n{out}"
        );
    }

    #[test]
    fn doc_instantiations_dedups_inhabited_per_type() {
        // PC and SP share `(BitVec 64)`: a single instance, first
        // constructor wins (matches sail-lake golden `register-v2.txt`).
        let out = small_info().doc_instantiations();
        assert!(
            out.contains("instance : Inhabited (RegisterRef (BitVec 64)) where"),
            "{out}"
        );
        assert!(out.contains("default := .Reg PC"), "{out}");
        assert!(!out.contains(".Reg SP"), "{out}");
        assert_eq!(out.matches("Inhabited (RegisterRef (BitVec 64))").count(), 1);
        assert!(
            out.contains("instance : Inhabited (RegisterRef (BitVec 32)) where"),
            "{out}"
        );
        assert!(out.contains("default := .Reg PSTATE"), "{out}");
        assert_eq!(RegisterInfo::empty().doc_instantiations(), "");
    }

    #[test]
    fn empty_registers_use_pempty_fallback() {
        let out = RegisterInfo::empty().doc_reg_info();
        assert!(out.contains("abbrev Register := PEmpty"), "{out}");
        assert!(out.contains("PEmpty.elim"), "{out}");
        assert_eq!(RegisterInfo::empty().regstate(false), "abbrev regstate := Unit\n");
    }

    #[test]
    fn read_write_mapping() {
        let info = small_info();
        assert_eq!(info.read_reg("PC").as_deref(), Some("readReg PC"));
        assert_eq!(
            info.write_reg("SP", "v").as_deref(),
            Some("writeReg SP v")
        );
        assert_eq!(info.read_reg("R0"), None);
        assert_eq!(info.write_reg("R0", "v"), None);
        // Arrow discipline (wrap_with_left_arrow (not as_monadic)):
        // monadic positions stay bare, pure positions gain `(← …)`.
        assert_eq!(
            info.read_reg_in("PC", true).as_deref(),
            Some("readReg PC")
        );
        assert_eq!(
            info.read_reg_in("PC", false).as_deref(),
            Some("(← readReg PC)")
        );
        assert_eq!(
            info.write_reg_in("SP", "v", true).as_deref(),
            Some("writeReg SP v")
        );
        assert_eq!(
            info.write_reg_in("SP", "v", false).as_deref(),
            Some("(← writeReg SP v)")
        );
        assert_eq!(info.read_reg_in("R0", false), None);
        assert_eq!(info.write_reg_in("R0", "v", false), None);
        assert_eq!(
            RegisterInfo::write_reg_ref("r", "v"),
            "writeRegRef r v"
        );
        assert_eq!(
            RegisterInfo::register_ref_ty("(BitVec 64)"),
            "(RegisterRef (BitVec 64))"
        );
    }

    #[test]
    fn regstate_grouped_and_ungrouped() {
        let info = small_info();
        let flat = info.regstate(false);
        assert!(flat.contains("structure regstate where"), "{flat}");
        assert!(flat.contains("  PC : (BitVec 64)"), "{flat}");
        assert!(flat.contains("  SP : (BitVec 64)"), "{flat}");
        let grouped = info.regstate(true);
        assert!(grouped.contains("bits_64_reg : String → (BitVec 64)"), "{grouped}");
        assert!(grouped.contains("bits_32_reg : String → (BitVec 32)"), "{grouped}");
        assert!(!grouped.contains("PC :"), "{grouped}");
    }

    #[test]
    fn initial_values_prefer_reset_sail() {
        let vals = small_info().initial_values();
        assert_eq!(vals.len(), 3);
        assert!(vals[0].from_reset);
        assert!(!vals[1].from_reset);
        assert_eq!(vals[1].value_sail, "0b".to_owned() + &"0".repeat(64));
    }

    #[test]
    fn initial_regstate_snippet_is_hard_error_not_sorry() {
        for grouped in [false, true] {
            let err = small_info()
                .initial_regstate_snippet(grouped, Span::dummy())
                .expect_err("non-empty regstate must be a hard error, never a placeholder stub");
            assert_eq!(err.code, DiagCode::ExpressionNotTranslatable);
            assert_eq!(err.code.code_str(), "E3512");
            let text = err.to_string();
            assert!(text.contains("E3512"), "{text}");
            assert!(text.contains("initial_regstate"), "{text}");
            assert!(!text.to_lowercase().contains("sorry"), "{text}");
        }
    }

    #[test]
    fn initial_regstate_snippet_empty_is_unit() {
        for grouped in [false, true] {
            let out = RegisterInfo::empty()
                .initial_regstate_snippet(grouped, Span::dummy())
                .expect("empty regstate is exactly translatable");
            assert!(out.contains("()"), "{out}");
            assert!(!out.to_lowercase().contains("sorry"), "{out}");
        }
    }

    #[test]
    fn lean_typ_subset() {
        assert_eq!(lean_of_sail_typ("bool"), "Bool");
        assert_eq!(lean_of_sail_typ("int"), "Int");
        assert_eq!(lean_of_sail_typ("bit"), "(BitVec 1)");
        assert_eq!(lean_of_sail_typ(" bits( 16 ) "), "(BitVec 16)");
        assert_eq!(lean_of_sail_typ("my_reg_t"), "my_reg_t");
    }
}
