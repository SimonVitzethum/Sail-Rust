#![forbid(unsafe_code)]
//! Bidirectional checking over the canonical AST using explicit work stacks.
//! Unsupported rules fail; no placeholder or unproved obligation is accepted.

use crate::{
    error::{TypeError, TypeErrorKind, TypeResult},
    program::{Program, ScatteredNamespace, Scheme},
};
use sail_ast::*;
use sail_solver::canonical::{Argument, Bindings, CanonicalQuery, Context, View};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

mod elab_snapshot;

#[derive(Clone, PartialEq, Eq, Debug)]
enum Step {
    Argument(usize),
    Result,
    Tuple(usize),
    TypeArgument(usize),
}
#[derive(Clone)]
struct Type {
    root: Arc<Scheme>,
    path: Vec<Step>,
    /// Rigid binder identity survives annotations with a different interner.
    scope: Arc<Scheme>,
    substitutions: Arc<Substitutions>,
    /// OCaml's implicit_to_int applies to function arguments, not returns.
    implicit_argument: bool,
}
#[derive(Clone)]
struct Scalar {
    owner: Type,
    index: usize,
}
impl Scalar {
    fn argument(&self) -> &TypArg {
        let TypKind::App { args, .. } = &self.owner.node().kind else {
            unreachable!("checked scalar path")
        };
        &args[self.index]
    }
    fn resolved(&self) -> Self {
        let mut scalar = self.clone();
        loop {
            let symbol = match &scalar.argument().kind {
                TypArgKind::NExp(Node {
                    kind: NExpKind::Var(symbol),
                    ..
                }) => *symbol,
                TypArgKind::Bool(c) if matches!(c.kind, ConstraintKind::Var(_)) => {
                    let ConstraintKind::Var(symbol) = c.kind else {
                        unreachable!()
                    };
                    symbol
                }
                _ => break,
            };
            let Some(bound) = scalar.owner.substitutions.get_scalar(symbol) else {
                break;
            };
            scalar = bound.clone();
        }
        scalar
    }
    fn view(&self) -> Argument<'_> {
        Argument {
            argument: self.argument(),
            context: self.owner.proof_context(),
        }
    }
}
/// Substitution storage as compact vectors, not tree maps: scopes and
/// substitutions hold zero to a handful of entries, and a B-tree node per
/// 10.000-deep loop/scope level retains tens of megabytes. Lookup stays a
/// linear scan over a few entries; order is deterministic insertion order.
#[derive(Default)]
struct Substitutions {
    types: Vec<(Symbol, Type)>,
    scalars: Vec<(Symbol, Scalar)>,
}
impl Substitutions {
    fn get_type(&self, symbol: Symbol) -> Option<&Type> {
        self.types
            .iter()
            .find(|(bound, _)| *bound == symbol)
            .map(|(_, typ)| typ)
    }
    fn get_scalar(&self, symbol: Symbol) -> Option<&Scalar> {
        self.scalars
            .iter()
            .find(|(bound, _)| *bound == symbol)
            .map(|(_, scalar)| scalar)
    }
}
impl Drop for Substitutions {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        for (_, mut typ) in std::mem::take(&mut self.types) {
            pending.push(std::mem::take(&mut typ.substitutions));
        }
        for (_, mut scalar) in std::mem::take(&mut self.scalars) {
            pending.push(std::mem::take(&mut scalar.owner.substitutions));
        }
        while let Some(bindings) = pending.pop() {
            if let Ok(mut bindings) = Arc::try_unwrap(bindings) {
                for (_, mut typ) in std::mem::take(&mut bindings.types) {
                    pending.push(std::mem::take(&mut typ.substitutions));
                }
                for (_, mut scalar) in std::mem::take(&mut bindings.scalars) {
                    pending.push(std::mem::take(&mut scalar.owner.substitutions));
                }
            }
        }
    }
}
static EMPTY_PROOF_VARS: BTreeMap<String, u64> = BTreeMap::new();
/// Explizite Frisch→Real-Abbildung für Obligations mit mehreren
/// Symboltabellen (B2: Index-Witness- gegen Funktions-Scope-Tabelle).
/// Frische Goal-Symbole (eigene Tabelle, kollisionsfrei) mappen auf reale
/// TypArgumente mit je eigenem Kontext; der Emitter folgt der Kette. Kein
/// Raten: fehlende Abbildung ist ein Fehler, nie ein Beweis.
struct GoalMap<'a> {
    map: Vec<(Symbol, Argument<'a>)>,
}
impl<'a> Bindings for GoalMap<'a> {
    fn resolve(&self, symbol: Symbol) -> Option<Argument<'a>> {
        let hit = self
            .map
            .iter()
            .find(|(bound, _)| *bound == symbol)
            .map(|(_, argument)| *argument);
        hit
    }
    fn rigid(&self, _name: &str) -> Option<(usize, Kind)> {
        None
    }
}
impl Bindings for Type {
    fn resolve(&self, symbol: Symbol) -> Option<Argument<'_>> {
        self.substitutions.get_scalar(symbol).map(Scalar::view)
    }
    fn rigid(&self, name: &str) -> Option<(usize, Kind)> {
        self.scope.typschm.quant.iter().find_map(|q| match q {
            QuantItem::Id(k) if self.scope.interner.resolve(k.kid) == Some(name) => {
                Some((Arc::as_ptr(&self.scope) as usize, k.kind))
            }
            _ => None,
        })
    }
}
impl Type {
    fn proof_context(&self) -> Context<'_> {
        Context {
            interner: &self.root.interner,
            variables: &EMPTY_PROOF_VARS,
            bindings: Some(self),
        }
    }
    fn root(root: Arc<Scheme>) -> Self {
        Self {
            scope: root.clone(),
            root,
            path: vec![],
            substitutions: Arc::default(),
            implicit_argument: false,
        }
    }
    fn node(&self) -> &Typ {
        let mut typ = &self.root.typschm.typ;
        for step in &self.path {
            typ = match (step, &typ.kind) {
                (Step::Argument(i), TypKind::Fn { args, .. }) => &args[*i],
                (Step::Result, TypKind::Fn { ret, .. }) => ret,
                (Step::Tuple(i), TypKind::Tuple(items)) => &items[*i],
                (Step::TypeArgument(i), TypKind::App { args, .. }) => {
                    let TypArgKind::Typ(typ) = &args[*i].kind else {
                        unreachable!()
                    };
                    typ
                }
                _ => unreachable!("checked type path"),
            };
        }
        typ
    }
    fn child(&self, step: Step) -> Self {
        let mut child = self.clone();
        child.implicit_argument = matches!(step, Step::Argument(_));
        child.path.push(step);
        child
    }
    fn resolved(&self) -> Self {
        let mut typ = self.clone();
        while let TypKind::Var(symbol) = typ.node().kind {
            let Some(bound) = typ.substitutions.get_type(symbol) else {
                break;
            };
            typ = bound.clone();
        }
        typ
    }
    fn owned(typ: Typ, it: Arc<SimpleInterner>) -> Self {
        Self::root(Arc::new(Scheme {
            typschm: TypSchm { quant: vec![], typ },
            interner: it,
        }))
    }
    fn name(&self) -> Option<&str> {
        match &self.node().kind {
            TypKind::Id(name) | TypKind::App { name, .. } => {
                self.root.interner.resolve(name.symbol).map(|name| {
                    if name == "implicit" && self.implicit_argument {
                        "atom"
                    } else {
                        name
                    }
                })
            }
            _ => None,
        }
    }
    fn label(&self) -> String {
        self.name()
            .map(|name| {
                match name {
                    "atom" => "int",
                    "atom_bool" => "bool",
                    name => name,
                }
                .to_owned()
            })
            .unwrap_or_else(|| match &self.node().kind {
                TypKind::Var(symbol) => self
                    .root
                    .interner
                    .resolve(*symbol)
                    .unwrap_or("<unbound>")
                    .into(),
                TypKind::Tuple(items) => format!("tuple of {} elements", items.len()),
                TypKind::Fn { .. } => "function".into(),
                TypKind::Bidir { .. } => "mapping".into(),
                TypKind::Exist { .. } => "existential type".into(),
                TypKind::Unknown => "unknown type".into(),
                TypKind::Id(_) | TypKind::App { .. } => "unbound type name".into(),
            })
    }
}
#[derive(Default, Clone)]
struct Scope {
    /// Per-scope locals as a compact vector (same rationale as
    /// Substitutions): scopes hold zero to a handful of entries.
    locals: Vec<(String, (Type, bool))>,
    /// A predicate is kept as a borrowed view of the condition's canonical
    /// type, including its call substitutions and rigid witness identities.
    facts: Vec<(Scalar, bool)>,
    parent: Option<Arc<Scope>>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(scope) = parent {
            match Arc::try_unwrap(scope) {
                Ok(mut scope) => parent = scope.parent.take(),
                Err(_) => break,
            }
        }
    }
}
impl Scope {
    fn branch(self: &Arc<Self>, condition: Option<&Type>, negated: bool) -> Arc<Self> {
        match condition.filter(|typ| typ.name() == Some("atom_bool")) {
            Some(condition) => Arc::new(Self {
                locals: Vec::new(),
                facts: vec![(
                    Scalar {
                        owner: condition.clone(),
                        index: 0,
                    },
                    negated,
                )],
                parent: Some(self.clone()),
            }),
            None => self.clone(),
        }
    }
    fn get(&self, name: &str) -> Option<&(Type, bool)> {
        let mut scope = Some(self);
        while let Some(current) = scope {
            if let Some(entry) = current.locals.iter().find(|(bound, _)| bound == name) {
                return Some(&entry.1);
            }
            scope = current.parent.as_deref();
        }
        None
    }
}
fn unsupported(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported(message.into()),
    }
}
fn arity(span: Span, expected: usize, found: usize) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::ArityMismatch { expected, found },
    }
}
fn mismatch(span: Span, expected: &Type, found: &Type) -> TypeError {
    TypeError::mismatch(span, expected.label(), found.label())
}

/// Whether an `atom_bool` condition carries its own assumption: a bare
/// variable (fresh `__value` witnesses from opening plain `bool`
/// results) links operand views only by arc identity, so comparison
/// synthesis still runs to build the real constraint. Real constraints
/// (`Lt`, `And`, …) need no duplicate.
fn is_vacuous_fact(typ: &Type) -> bool {
    let TypKind::App { args, .. } = &typ.node().kind else {
        return false;
    };
    if typ.name() != Some("atom_bool") {
        return false;
    };
    let Some(arg) = args.first() else {
        return false;
    };
    matches!(
        &arg.kind,
        TypArgKind::Bool(c) if matches!(&c.kind, ConstraintKind::Var(_))
    )
}

/// Limit direct sibling-guided synthesis; deeper call chains use the
/// iterative worklist. Small-stack stress tests cover the fallback.
const SYNTH_DIRECT_LIMIT: u32 = 16;

struct Checker<'a> {
    program: &'a Program,
    it: Arc<SimpleInterner>,
    position: usize,
    builtins: BTreeMap<&'static str, Type>,
    /// Shared refined singleton types (`atom(0)`, `bitvector(2)`, ...).
    /// Literals and vector lengths rebuild identical shapes per occurrence;
    /// sharing one root per constant avoids a fresh Scheme/interner per read
    /// while every clone stays an independent view (as with builtins).
    refined_cache: HashMap<(String, String), Type>,
    /// Shared `from <= index <= to` condition shape for foreach scopes.
    /// Only the per-level substitutions (bound owners) differ; sharing one
    /// root/interner avoids rebuilding an identical constraint AST per loop
    /// in deep programs. Every level still gets its own rigid index witness.
    foreach_template: Option<ForeachTemplate>,
    expressions: usize,
    context: Option<Arc<Scheme>>,
    /// Per-definition synthesis cache; pointers are identity keys only and
    /// are never dereferenced. The loaded AST outlives this checker.
    inferred: HashMap<*const Exp, InferredValue>,
    calls: HashMap<*const Exp, Type>,
    /// Scheme of the candidate the last `select_call` picked.
    picked: std::cell::RefCell<Option<Arc<Scheme>>>,
    proof_scope: Arc<Scope>,
    /// Type variables bound by `P_var` patterns (`let 'm = e`), as numeric
    /// witness views; a later binding of the same symbol shadows an older
    /// one. They are not rigid function quantifiers: proofs substitute the
    /// witness of the bound value.
    tyvars: Vec<(Symbol, Scalar)>,
    /// Types of visited nodes, when elaborating.
    recorder: Option<elab_snapshot::Recorder>,
    /// Rigid witnesses met while grounding lengths to Mini numerics, under
    /// unique names, so obligations can link them back to their binders.
    witness_links: std::cell::RefCell<Vec<(String, Scalar)>>,
    /// Set while a condition is used only positively (assertions).
    weak_facts: std::cell::Cell<bool>,
    /// Name of the call being resolved, for diagnostics only (expression
    /// spans are definition-wide).
    call_hint: std::cell::RefCell<String>,
    /// Direct-synthesis nesting depth (see `SYNTH_DIRECT_LIMIT`).
    synth_depth: u32,
}
#[derive(Clone)]
struct ForeachTemplate {
    condition: Type,
    lower: Symbol,
    index: Symbol,
    upper: Symbol,
}
struct InferredValue {
    value: Type,
    /// Declared type shared via Arc. Opening a plain scalar is useful for
    /// proofs but must not narrow a mutable declaration, so both views are
    /// kept: readers share the declared allocation (refcount bump) instead
    /// of cloning Type views and substitution chains per read.
    declared: Arc<Type>,
}
struct Task<'a> {
    exp: &'a Exp,
    expected: Type,
    scope: Arc<Scope>,
    returning: Type,
}

// Lane-54 / INSERT-37A: body-time view adapter. Bridges the body pass over
// `Program` to `scattered_elab::ScatteredView` using the source-position
// accessors (`scattered` / `scattered_clause_count` / `scattered_closed_at`;
// the latter plus the `From` mapping arrive via the accessor patch).
struct ProgramScattered<'a> {
    program: &'a Program,
    position: usize,
}
impl crate::scattered_elab::ScatteredView for ProgramScattered<'_> {
    fn is_open(&self, ns: crate::scattered_elab::Namespace, name: &str) -> bool {
        self.program
            .scattered(ns.into(), name, self.position)
            .is_some_and(|decl| !decl.closed)
    }
    fn clause_count(&self, ns: crate::scattered_elab::Namespace, name: &str) -> usize {
        self.program
            .scattered_clause_count(ns.into(), name, self.position)
    }
    fn closed_at(&self, name: &str, position: usize) -> Option<crate::scattered_elab::Namespace> {
        // `scattered_closed_at` returns `program::ScatteredNamespace`; map
        // back variant-for-variant (same four variants, same order).
        self.program
            .scattered_closed_at(name, position)
            .map(|ns| match ns {
                ScatteredNamespace::Function => crate::scattered_elab::Namespace::Function,
                ScatteredNamespace::Mapping => crate::scattered_elab::Namespace::Mapping,
                ScatteredNamespace::Variant => crate::scattered_elab::Namespace::Variant,
                ScatteredNamespace::Enum => crate::scattered_elab::Namespace::Enum,
            })
    }
}

impl<'a> Checker<'a> {
    fn new(program: &'a Program, it: Arc<SimpleInterner>, position: usize) -> Self {
        Checker {
        program,
        it,
        position,
        builtins: BTreeMap::new(),
        refined_cache: HashMap::new(),
        foreach_template: None,
        expressions: 0,
        context: None,
        inferred: HashMap::new(),
        calls: HashMap::new(),
        picked: std::cell::RefCell::new(None),
        proof_scope: Arc::default(),
        tyvars: Vec::new(),
        recorder: None,
        witness_links: std::cell::RefCell::new(Vec::new()),
        weak_facts: std::cell::Cell::new(false),
        call_hint: std::cell::RefCell::new(String::new()),
        synth_depth: 0,
    }
    }
}

/// Types of the names bound by an unannotated global `let`, inferred from its
/// initializer (OCaml `check_letdef`). Only types that do not depend on call
/// substitutions snapshot exactly; numeric ones fall back to `int`.
pub(crate) fn infer_global_let(
    program: &Program,
    def: &Def,
    it: Arc<SimpleInterner>,
    position: usize,
) -> TypeResult<Vec<(String, Arc<Scheme>)>> {
    let DefKind::Let { pat, exp } = &def.kind else {
        return Ok(Vec::new());
    };
    // Annotated bindings are declared with their annotation already.
    if matches!(pat.kind, PatKind::Typ { .. }) {
        return Ok(Vec::new());
    }
    let mut checker = Checker::new(program, it, position);
    let scope = Arc::new(Scope::default());
    let typ = checker.infer(exp, &scope)?;
    let mut locals = Scope::default();
    checker.pattern(pat, typ, false, &mut locals)?;
    let mut bound = Vec::new();
    for (name, (typ, _)) in locals.locals.clone() {
        let typ = checker.closed_snapshot(&typ, def.span)?;
        let node = typ.node().clone();
        bound.push((
            name,
            Arc::new(Scheme {
                typschm: TypSchm {
                    quant: Vec::new(),
                    typ: node,
                },
                interner: typ.root.interner.clone(),
            }),
        ));
    }
    Ok(bound)
}

pub(crate) fn check(
    program: &Program,
    def: &Def,
    it: Arc<SimpleInterner>,
    position: usize,
) -> TypeResult<usize> {
    check_with(program, def, it, position, false).map(|checker| checker.expressions)
}

/// Check a definition and annotate its expressions and patterns with their
/// elaborated types (`Annotation::typ`). Returns the interner extended with
/// the names the snapshots introduced.
pub(crate) fn elaborate(
    program: &Program,
    def: &mut Def,
    it: Arc<SimpleInterner>,
    position: usize,
) -> TypeResult<SimpleInterner> {
    let checker = check_with(program, &*def, it.clone(), position, true)?;
    let mut checker = checker;
    // Expressions that were only inferred (conditions, assert operands, ...)
    // never became tasks; their inferred type is the best annotation.
    let inferred_types: Vec<(usize, Type)> = checker
        .inferred
        .iter()
        .map(|(address, inferred)| (*address as usize, (*inferred.declared).clone()))
        .collect();
    if let Some(recorder) = checker.recorder.as_mut() {
        for (address, typ) in inferred_types {
            recorder.exps.entry(address).or_insert(typ);
        }
    }
    let recorder = checker.recorder.as_ref().expect("recording checker");
    let mut out = (*it).clone();
    let mut exp_types: HashMap<usize, Typ> = HashMap::with_capacity(recorder.exps.len());
    for (address, typ) in &recorder.exps {
        exp_types.insert(*address, checker.snapshot_typ(typ, &mut out));
    }
    let mut pat_types: HashMap<usize, Typ> = HashMap::with_capacity(recorder.pats.len());
    for (address, typ) in &recorder.pats {
        pat_types.insert(*address, checker.snapshot_typ(typ, &mut out));
    }
    struct Attach<'o> {
        out: &'o mut SimpleInterner,
        callees: HashMap<usize, String>,
        exps: HashMap<usize, Typ>,
        pats: HashMap<usize, Typ>,
        lexps: HashMap<usize, Typ>,
        intros: std::collections::HashSet<usize>,
    }
    impl elab_snapshot::Visit for Attach<'_> {
        fn exp(&mut self, exp: &mut Exp) {
            let address = exp as *const Exp as usize;
            if std::env::var_os("SAIL_RS_DEBUG_CALLS").is_some() {
                if let ExpKind::App { func, .. } = &exp.kind {
                    eprintln!("attach app {:?} at {address:x}: {:?}", self.out.resolve(func.symbol), self.callees.get(&address));
                }
            }
            if let Some(member) = self.callees.remove(&address) {
                if let ExpKind::App { func, .. } = &mut exp.kind {
                    func.symbol = self.out.intern(&member);
                    func.is_operator = false;
                }
            }
            if let Some(typ) = self.exps.remove(&address) {
                exp.annotation_mut().typ = Some(Box::new(typ));
            }
            if self.intros.contains(&address) {
                exp.annotation_mut().attributes.push(Attribute {
                    name: "intro".into(),
                    data: None,
                });
            }
        }
        fn pat(&mut self, pat: &mut Pat) {
            if let Some(typ) = self.pats.remove(&(pat as *const Pat as usize)) {
                pat.annotation_mut().typ = Some(Box::new(typ));
            }
        }
        fn lexp(&mut self, lexp: &mut LExp) {
            if let Some(typ) = self.lexps.remove(&(lexp as *const LExp as usize)) {
                lexp.annotation_mut().typ = Some(Box::new(typ));
            }
        }
    }
    let mut lexp_types: HashMap<usize, Typ> = HashMap::with_capacity(recorder.lexps.len());
    for (address, typ) in &recorder.lexps {
        lexp_types.insert(*address, checker.snapshot_typ(typ, &mut out));
    }
    let intros = recorder.intros.clone();
    let callees = recorder.callees.clone();
    drop(checker);
    elab_snapshot::walk_def_mut(
        def,
        &mut Attach {
            out: &mut out,
            callees,
            exps: exp_types,
            pats: pat_types,
            lexps: lexp_types,
            intros,
        },
    );
    Ok(out)
}

fn check_with<'a>(
    program: &'a Program,
    def: &Def,
    it: Arc<SimpleInterner>,
    position: usize,
    record: bool,
) -> TypeResult<Checker<'a>> {
    let mut checker = Checker::new(program, it, position);
    if record {
        checker.recorder = Some(elab_snapshot::Recorder::default());
    }
    match &def.kind {
        DefKind::Fundef { rec, funcls, .. } => {
            program.check_function_header(def, &checker.it, position)?;
            let measure = match rec {
                RecOpt::Measure { pat, exp } => Some((pat as &Pat, exp as &Exp)),
                _ => None,
            };
            for (index, funcl) in funcls.iter().enumerate() {
                let own = if index == 0 { measure } else { None };
                checker.function(funcl, own)?;
            }
        }
        // Lane-54 / INSERT-37B: scattered bodies elaborate against the
        // scattered tables instead of the blanket `Unsupported`. Headers,
        // enum members and ends carry no body work; mapping/union clauses
        // keep a precise `Unsupported` until their elaboration lands.
        // Block openings were validated by `declare_scattered` (their own
        // declaration is visible at this position, so re-running the
        // open-block rule here would reject every header).
        DefKind::Scattered(
            ScatteredDef::Function { .. }
            | ScatteredDef::Mapping { .. }
            | ScatteredDef::Variant { .. }
            | ScatteredDef::Enum(_),
        ) => {}
        DefKind::Scattered(scat) => {
            use crate::scattered_elab::BodyPlan;
            let view = ProgramScattered { program, position };
            match crate::scattered_elab::elaborate(def, &checker.it, &view, position)? {
                BodyPlan::AcceptHeader { .. }
                | BodyPlan::AcceptEnumcl { .. }
                | BodyPlan::AcceptEnd { .. } => {}
                BodyPlan::CheckFuncl { .. } => {
                    let ScatteredDef::Funcl(funcl) = scat else {
                        return Err(unsupported(def.span, "scattered plan/clause mismatch"));
                    };
                    // INSERT-37C (follow-up, see 37C-SPEC): `checker.function`
                    // resolves the signature via `program.value`, but scattered
                    // functions keep their header in the scattered table, not
                    // the value table. Until 37C lands this keeps working for
                    // headers mirrored into the value table and reports the
                    // precise `Unsupported` below otherwise.
                    checker.function(funcl, None)?;
                }
                BodyPlan::CheckMapcl { .. } | BodyPlan::CheckUnioncl { .. } => {
                    return Err(unsupported(
                        def.span,
                        "scattered mapping/union clause bodies (mapping plans / union payload elaboration pending)",
                    ))
                }
            }
        }
        DefKind::Measure { name, pat, exp } => {
            let args = crate::measure_check::check_measure_target(
                program,
                name,
                &checker.it,
                position,
                def.span,
            )?;
            // The measure sees the function's type variables (`= 'N`).
            let function = checker.name(name)?.to_owned();
            checker.context = program.value(&function, position).cloned();
            checker.measure(pat, exp, &args)?;
        }
        DefKind::LoopMeasures { name, measures } => {
            let count = crate::measure_check::check_loop_measures_target(
                program,
                name,
                &checker.it,
                measures,
                position,
                def.span,
            )?;
            debug_assert_eq!(count, measures.len());
            checker.loop_measures(measures)?;
        }
        DefKind::Outcome {
            typschm,
            quant: params,
            defs,
            ..
        } => {
            // Outcome parameters are rigid throughout the block. Header
            // quantifiers additionally scope over each implementation, while
            // local vals retain only their own polymorphic quantifiers.
            let scheme = Arc::new(Scheme {
                typschm: TypSchm {
                    quant: params.iter().chain(&typschm.quant).cloned().collect(),
                    typ: typschm.typ.clone(),
                },
                interner: checker.it.clone(),
            });
            let mut locals = Arc::new(Scope::default());
            for inner in defs {
                match &inner.kind {
                    DefKind::Val(val) => {
                        let mut typ = Type::root(Arc::new(Scheme {
                            typschm: val.typschm.clone(),
                            interner: checker.it.clone(),
                        }));
                        typ.scope = scheme.clone();
                        locals = Arc::new(Scope {
                            locals: vec![(checker.name(&val.name)?.to_owned(), (typ, false))],
                            facts: Vec::new(),
                            parent: Some(locals),
                        });
                    }
                    DefKind::Impl(funcl) => checker.function_with_type(
                        funcl,
                        None,
                        Type::root(scheme.clone()),
                        Some(locals.clone()),
                    )?,
                    _ => {
                        return Err(unsupported(
                            inner.span,
                            "forbidden definition in outcome block",
                        ))
                    }
                }
            }
        }
        DefKind::Impl(_) => return Err(unsupported(def.span, "implementation outside an outcome")),
        // Validated when the instantiation is declared (parameters, outcome
        // constraints, required functions); nothing is left to check here.
        DefKind::Instantiation { .. } => {}
        DefKind::Mapdef { .. } | DefKind::InternalMutrec(_) => {
            return Err(unsupported(
                def.span,
                "mapping, scattered, outcome or measure definition",
            ))
        }
        DefKind::Let { pat, exp } => {
            let scope = Arc::new(Scope::default());
            let typ = if let PatKind::Typ { typ, .. } = &pat.kind {
                checker.annotation(typ)?
            } else {
                checker.infer(exp, &scope)?
            };
            let mut locals = Scope::default();
            checker.pattern(pat, typ.clone(), false, &mut locals)?;
            checker.run(vec![Task {
                exp,
                expected: typ.clone(),
                scope,
                returning: typ,
            }])?;
        }
        DefKind::Register(register) => {
            if let Some(exp) = &register.init {
                let typ = Type::owned(register.typ.clone(), checker.it.clone());
                checker.run(vec![Task {
                    exp,
                    expected: typ.clone(),
                    scope: Arc::new(Scope::default()),
                    returning: typ,
                }])?;
            }
        }
        DefKind::Type(_)
        | DefKind::Val(_)
        | DefKind::Constraint(_)
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Pragma { .. } => {}
    }
    Ok(checker)
}

impl Checker<'_> {
    /// Expand type synonyms using borrowed argument views. Only declarations
    /// preceding this body are visible. Declaration checking forbids recursive
    /// synonyms, so each expansion descends a finite type/alias dependency tree.
    fn resolve(&self, typ: &Type) -> TypeResult<Type> {
        let mut typ = typ.resolved();
        while let Some(synonym) = typ
            .name()
            .and_then(|name| self.program.type_synonym(name, self.position))
        {
            let args = match &typ.node().kind {
                TypKind::Id(_) => &[][..],
                TypKind::App { args, .. } => args.as_slice(),
                _ => unreachable!("named synonym"),
            };
            let binders: Vec<_> = synonym
                .typschm
                .quant
                .iter()
                .filter_map(|q| {
                    if let QuantItem::Id(k) = q {
                        Some(k)
                    } else {
                        None
                    }
                })
                .collect();
            if binders.len() != args.len() {
                return Err(arity(typ.node().span, binders.len(), args.len()));
            }
            let mut expanded = Type::root(synonym.clone());
            expanded.scope = typ.scope.clone();
            expanded.implicit_argument = typ.implicit_argument;
            let mut types = Vec::new();
            let mut scalars = Vec::new();
            for (index, (binder, arg)) in binders.into_iter().zip(args).enumerate() {
                match (binder.kind, &arg.kind) {
                    (Kind::Type, TypArgKind::Typ(_)) => {
                        types.push((binder.kid, typ.child(Step::TypeArgument(index))));
                    }
                    (Kind::Int, TypArgKind::NExp(_)) | (Kind::Bool, TypArgKind::Bool(_)) => {
                        scalars.push((
                            binder.kid,
                            Scalar {
                                owner: typ.clone(),
                                index,
                            }
                            .resolved(),
                        ));
                    }
                    _ => {
                        return Err(unsupported(
                            typ.node().span,
                            "type synonym argument kind mismatch",
                        ))
                    }
                }
            }
            expanded.substitutions = Arc::new(Substitutions { types, scalars });
            typ = expanded.resolved();
        }
        Ok(typ)
    }
    /// Open a plain scalar value into a fresh rigid singleton witness. Each
    /// immutable binding or expression occurrence keeps its own witness; two
    /// unrelated integers/booleans never share a numeric type variable.
    /// Takes the declared view by reference so caching it in an Arc does not
    /// clone Type paths or substitution chains just to open a value.
    fn open_value(&self, typ: &Type) -> Type {
        let (constructor, kind) = match (&typ.node().kind, typ.name()) {
            (TypKind::Id(_), Some("int")) => ("atom", Kind::Int),
            (TypKind::Id(_), Some("bool")) => ("atom_bool", Kind::Bool),
            _ => return typ.clone(),
        };
        let mut interner = SimpleInterner::new();
        let name = Name::dummy(interner.intern(constructor));
        let kid = interner.intern("__value");
        let argument = if kind == Kind::Int {
            TypArgKind::NExp(Node::dummy(NExpKind::Var(kid)))
        } else {
            TypArgKind::Bool(Box::new(Node::dummy(ConstraintKind::Var(kid))))
        };
        Type::root(Arc::new(Scheme {
            typschm: TypSchm {
                quant: vec![QuantItem::Id(KOpt { kind, kid })],
                typ: Node::dummy(TypKind::App {
                    name,
                    args: vec![Node::dummy(argument)],
                }),
            },
            interner: Arc::new(interner),
        }))
    }
    /// OCaml `bind_pat` flow typing for literals: matching `atom(n)` against
    /// a numeric literal (or `atom_bool(c)` against `true`/`false`) assumes
    /// the equality in the arm instead of proving it. Returns whether the
    /// literal was handled this way.
    fn flow_literal(&mut self, lit: &Lit, typ: &Type, scope: &mut Scope) -> TypeResult<bool> {
        let TypKind::App { args, .. } = &typ.node().kind else {
            return Ok(false);
        };
        if args.len() != 1 {
            return Ok(false);
        }
        match (&lit.kind, typ.name(), &args[0].kind) {
            (LitKind::Num(symbol), Some("atom"), TypArgKind::NExp(_)) => {
                let Some(value) = self.it.resolve(*symbol) else {
                    return Ok(false);
                };
                let mut it = SimpleInterner::new();
                let placeholder = it.intern("__scrutinee");
                let atom_bool = Name::dummy(it.intern("atom_bool"));
                let equal = Node::dummy(ConstraintKind::Equal(
                    Box::new(Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(
                        placeholder,
                    ))))),
                    Box::new(Node::dummy(TypArgKind::NExp(Node::dummy(
                        NExpKind::Constant(value.to_owned()),
                    )))),
                ));
                let mut fact = Type::root(Arc::new(Scheme {
                    typschm: TypSchm {
                        quant: Vec::new(),
                        typ: Node::dummy(TypKind::App {
                            name: atom_bool,
                            args: vec![Node::dummy(TypArgKind::Bool(Box::new(equal)))],
                        }),
                    },
                    interner: Arc::new(it),
                }));
                fact.substitutions = Arc::new(Substitutions {
                    types: Vec::new(),
                    scalars: vec![(
                        placeholder,
                        Scalar {
                            owner: typ.clone(),
                            index: 0,
                        }
                        .resolved(),
                    )],
                });
                scope.facts.push((Scalar { owner: fact, index: 0 }, false));
                Ok(true)
            }
            (LitKind::True | LitKind::False, Some("atom_bool"), TypArgKind::Bool(_)) => {
                scope.facts.push((
                    Scalar {
                        owner: typ.clone(),
                        index: 0,
                    },
                    matches!(lit.kind, LitKind::False),
                ));
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// OCaml `bind_typ_pat`: bind pattern variables to the numeric
    /// arguments of the matched type (`atom('n)` gives `'n`).
    fn bind_type_pattern(&mut self, pattern: &TypPat, typ: &Type) -> TypeResult<()> {
        let mut work = vec![(pattern, typ.resolved())];
        while let Some((pattern, typ)) = work.pop() {
            match &pattern.kind {
                TypPatKind::Wild => {}
                TypPatKind::Var(symbol) => {
                    let TypKind::App { args, .. } = &typ.node().kind else {
                        return Err(unsupported(pattern.span, "type variable pattern against a non-numeric type"));
                    };
                    if args.len() != 1 || !matches!(args[0].kind, TypArgKind::NExp(_)) {
                        return Err(unsupported(
                            pattern.span,
                            "type variable pattern against a non-numeric type",
                        ));
                    }
                    let witness = Scalar { owner: typ.clone(), index: 0 }.resolved();
                    self.tyvars.retain(|(bound, _)| bound != symbol);
                    self.tyvars.push((*symbol, witness));
                }
                TypPatKind::App { args: patterns, .. } => {
                    let TypKind::App { args, .. } = &typ.node().kind else {
                        return Err(unsupported(pattern.span, "type pattern against a non-application type"));
                    };
                    if args.len() != patterns.len() {
                        return Err(arity(pattern.span, args.len(), patterns.len()));
                    }
                    for (index, sub) in patterns.iter().enumerate() {
                        match (&sub.kind, &args[index].kind) {
                            (TypPatKind::Wild, _) => {}
                            (TypPatKind::Var(symbol), TypArgKind::NExp(_)) => {
                                let witness = Scalar { owner: typ.clone(), index }.resolved();
                                self.tyvars.retain(|(bound, _)| bound != symbol);
                                self.tyvars.push((*symbol, witness));
                            }
                            (_, TypArgKind::Typ(_)) => {
                                work.push((sub, typ.child(Step::TypeArgument(index))));
                            }
                            _ => {
                                return Err(unsupported(sub.span, "type pattern argument kind"));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    /// `{'j, 'j == a | 'j == b. atom('j)}` for two numeric singleton views.
    fn join_atoms(&self, left: &Type, right: &Type) -> Option<Type> {
        let numeric = |typ: &Type| {
            typ.name() == Some("atom")
                && matches!(
                    &typ.node().kind,
                    TypKind::App { args, .. }
                        if args.len() == 1 && matches!(args[0].kind, TypArgKind::NExp(_))
                )
        };
        if !numeric(left) || !numeric(right) {
            return None;
        }
        let mut it = SimpleInterner::new();
        let (a, b, joined) = (it.intern("__a"), it.intern("__b"), it.intern("__join"));
        let atom = Name::dummy(it.intern("atom"));
        let var = |symbol| Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(symbol))));
        let equal = |other| {
            Node::dummy(ConstraintKind::Equal(
                Box::new(var(joined)),
                Box::new(var(other)),
            ))
        };
        let typ = Node::dummy(TypKind::Exist {
            binders: vec![KOpt {
                kind: Kind::Int,
                kid: joined,
            }],
            constraint: Box::new(Node::dummy(ConstraintKind::Or(
                Box::new(equal(a)),
                Box::new(equal(b)),
            ))),
            typ: Box::new(Node::dummy(TypKind::App {
                name: atom,
                args: vec![var(joined)],
            })),
        });
        let mut joined_type = Type::root(Arc::new(Scheme {
            typschm: TypSchm {
                quant: Vec::new(),
                typ,
            },
            interner: Arc::new(it),
        }));
        joined_type.substitutions = Arc::new(Substitutions {
            types: Vec::new(),
            scalars: vec![
                (
                    a,
                    Scalar {
                        owner: left.clone(),
                        index: 0,
                    }
                    .resolved(),
                ),
                (
                    b,
                    Scalar {
                        owner: right.clone(),
                        index: 0,
                    }
                    .resolved(),
                ),
            ],
        });
        Some(joined_type)
    }
    /// `int`/`bool` when `typ` is a fresh witness made by `open_value`.
    fn opened_value_base(&self, typ: &Type) -> Option<&'static str> {
        if !typ.path.is_empty() || typ.root.typschm.quant.len() != 1 {
            return None;
        }
        let QuantItem::Id(binder) = &typ.root.typschm.quant[0] else {
            return None;
        };
        if typ.root.interner.resolve(binder.kid) != Some("__value") {
            return None;
        }
        match binder.kind {
            Kind::Int => Some("int"),
            Kind::Bool => Some("bool"),
            Kind::Type => None,
        }
    }
    fn value_type(&mut self, exp: &Exp, declared: Type) -> Type {
        if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
            return typ.value.clone();
        }
        // Existential results open once here (memoized: stable binder
        // identities for every later use of this expression).
        let opened = self.open_existential(&declared);
        let typ = self.open_value(&opened);
        let shared: Arc<Type> = Arc::new(declared);
        self.inferred.insert(
            exp as *const Exp,
            InferredValue {
                value: typ.clone(),
                declared: shared,
            },
        );
        typ
    }
    fn proof_assumptions<'a>(&'a self, scope: &'a Option<Type>) -> Vec<View<'a>> {
        let mut assumptions: Vec<_> = self
            .program
            .constraints(self.position)
            .map(|(constraint, it)| View {
                negated: false,
                constraint,
                context: Context {
                    interner: it,
                    variables: &EMPTY_PROOF_VARS,
                    bindings: None,
                },
            })
            .collect();
        if let Some(scope) = scope {
            assumptions.extend(scope.root.typschm.quant.iter().filter_map(|q| {
                if let QuantItem::Constraint(c) = q {
                    Some(View {
                        negated: false,
                        constraint: c,
                        context: scope.proof_context(),
                    })
                } else {
                    None
                }
            }));
        }
        let mut current = Some(self.proof_scope.as_ref());
        while let Some(scope) = current {
            for (scalar, negated) in &scope.facts {
                let TypArgKind::Bool(constraint) = &scalar.argument().kind else {
                    unreachable!("Boolean branch fact")
                };
                assumptions.push(View {
                    constraint,
                    context: scalar.owner.proof_context(),
                    negated: *negated,
                });
            }
            current = scope.parent.as_deref();
        }
        assumptions
    }
    /// Types owning the facts of the current proof scope; their opened
    /// existential bounds (markers) are evidence for every later proof.
    fn fact_owners(&self) -> Vec<&Type> {
        let mut owners = Vec::new();
        let mut scope = Some(&*self.proof_scope);
        while let Some(current) = scope {
            owners.extend(current.facts.iter().map(|(fact, _)| &fact.owner));
            scope = current.parent.as_deref();
        }
        owners
    }
    fn scalar_equal(&self, left: &Scalar, right: &Scalar, span: Span) -> TypeResult<()> {
        let left = left.resolved();
        let right = right.resolved();
        if left.index == right.index
            && left.owner.path == right.owner.path
            && Arc::ptr_eq(&left.owner.root, &right.owner.root)
            && Arc::ptr_eq(&left.owner.scope, &right.owner.scope)
            && Arc::ptr_eq(&left.owner.substitutions, &right.owner.substitutions)
        {
            return Ok(());
        }
        if left.owner.substitutions.scalars.is_empty()
            && right.owner.substitutions.scalars.is_empty()
            && crate::canonical_eq::argument(
                left.argument(),
                right.argument(),
                &left.owner.root.interner,
                &right.owner.root.interner,
                Arc::ptr_eq(&left.owner.scope, &right.owner.scope),
            )
        {
            return Ok(());
        }
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let mut assumptions = self.proof_assumptions(&scope);
        assumptions.extend(self.marker_assumptions(&[&left.owner, &right.owner]));
        assumptions.extend(self.marker_assumptions(&self.fact_owners()));
        let query = CanonicalQuery::equality(&assumptions, left.view(), right.view())
            .map_err(|e| unsupported(span, e))?;
        if !self.program.prove(query).is_valid() {
            return Err(TypeError::constraint_not_proved(
                span,
                "dependent type argument equality",
            ));
        }
        Ok(())
    }
    fn annotation(&self, typ: &Typ) -> TypeResult<Type> {
        if self.tyvars.is_empty() {
            self.program
                .check_annotation(typ, &self.it, self.position, self.context.as_deref())?;
        } else {
            // Local type variables are in scope for well-formedness only;
            // proofs see them through the witness substitution below.
            let mut interner = self
                .context
                .as_ref()
                .map_or_else(|| (*self.it).clone(), |scope| (*scope.interner).clone());
            let mut quant = self
                .context
                .as_ref()
                .map(|scope| scope.typschm.quant.clone())
                .unwrap_or_default();
            let shadowed: Vec<&str> = self
                .tyvars
                .iter()
                .filter_map(|(symbol, _)| self.it.resolve(*symbol))
                .collect();
            quant.retain(|item| match item {
                QuantItem::Id(binder) => self
                    .context
                    .as_ref()
                    .and_then(|scope| scope.interner.resolve(binder.kid))
                    .is_none_or(|name| !shadowed.contains(&name)),
                QuantItem::Constraint(_) => true,
            });
            for name in &shadowed {
                quant.push(QuantItem::Id(KOpt {
                    kind: Kind::Int,
                    kid: interner.intern(name),
                }));
            }
            let extended = Scheme {
                typschm: TypSchm {
                    quant,
                    typ: Node::dummy(TypKind::Unknown),
                },
                interner: Arc::new(interner),
            };
            self.program
                .check_annotation(typ, &self.it, self.position, Some(&extended))?;
        }
        let (expanded, interner) = self.program.expanded_typ(typ, &self.it, self.position);
        let mut typ = Type::owned(expanded, interner);
        if let Some(scope) = &self.context {
            typ.scope = scope.clone();
        }
        if !self.tyvars.is_empty() {
            typ.substitutions = Arc::new(Substitutions {
                types: Vec::new(),
                scalars: self.tyvars.clone(),
            });
        }
        Ok(typ)
    }
    fn builtin(&mut self, name: &'static str) -> Type {
        self.builtins
            .entry(name)
            .or_insert_with(|| {
                let mut it = SimpleInterner::new();
                let symbol = it.intern(name);
                Type::owned(Node::dummy(TypKind::Id(Name::dummy(symbol))), Arc::new(it))
            })
            .clone()
    }
    fn name(&self, name: &Name) -> TypeResult<&str> {
        self.it
            .resolve(name.symbol)
            .ok_or_else(|| TypeError::unbound_var(name.span, format!("symbol {}", name.symbol)))
    }
    fn id(&self, name: &Name, scope: &Scope) -> TypeResult<Type> {
        let name_text = self.name(name)?;
        if let Some((typ, _)) = scope.get(name_text) {
            return Ok(typ.clone());
        }
        self.program
            .register(name_text, self.position)
            .or_else(|| self.program.value(name_text, self.position))
            .cloned()
            .map(Type::root)
            .ok_or_else(|| TypeError::unbound_var(name.span, name_text.to_owned()))
    }
    /// OCaml `infer_funapp'`: `f()` calls a function whose formals are all
    /// `implicit(..)` without explicit arguments.
    fn effective_args<'a>(candidates: &[Type], args: &'a [Exp]) -> &'a [Exp] {
        let [only] = args else {
            return args;
        };
        let ExpKind::Lit(lit) = &only.kind else {
            return args;
        };
        if !matches!(lit.kind, LitKind::Unit) || candidates.is_empty() {
            return args;
        }
        let implicit_only = candidates.iter().all(|callee| match &callee.node().kind {
            TypKind::Fn { args: formals, .. } => {
                !formals.is_empty()
                    && formals.iter().all(|formal| Self::is_implicit_formal(callee, formal))
            }
            _ => false,
        });
        if implicit_only {
            &args[..0]
        } else {
            args
        }
    }
    /// A type without free call substitutions, for storing in the frozen
    /// declaration tables: numeric widths and lengths are grounded to
    /// constants; refinements that stay symbolic widen or fail honestly.
    fn closed_snapshot(&mut self, typ: &Type, span: Span) -> TypeResult<Type> {
        let typ = self.resolve(typ)?;
        let plain = typ.substitutions.scalars.is_empty() && typ.substitutions.types.is_empty();
        match (&typ.node().kind, typ.name()) {
            (TypKind::Tuple(items), _) => {
                let mut parts = Vec::with_capacity(items.len());
                for index in 0..items.len() {
                    parts.push(self.closed_snapshot(&typ.child(Step::Tuple(index)), span)?);
                }
                Ok(self.tuple_type(parts))
            }
            (_, Some("atom")) if !plain => Ok(self.builtin("int")),
            (_, Some("atom_bool")) if !plain => Ok(self.builtin("bool")),
            (TypKind::App { .. }, Some("bitvector" | "bits")) if !plain => {
                let len = self
                    .vector_len_nexp(&typ, span)
                    .ok()
                    .as_ref()
                    .and_then(crate::nexp::Nexp::as_const)
                    .ok_or_else(|| unsupported(span, "global binding has a symbolic vector length"))?;
                Ok(self.refined(
                    "bitvector",
                    TypArgKind::NExp(Node::dummy(NExpKind::Constant(len.to_string()))),
                ))
            }
            _ if plain => Ok(typ),
            _ => Err(unsupported(
                span,
                "global binding has a type that depends on call substitutions",
            )),
        }
    }
    fn set_call_hint(&self, name: &Name) {
        if let Ok(text) = self.name(name) {
            *self.call_hint.borrow_mut() = text.to_owned();
        }
    }
    fn call_candidates(
        &self,
        name: &Name,
        scope: &Scope,
        arg_count: usize,
    ) -> TypeResult<Vec<Type>> {
        let name_text = self.name(name)?;
        // Lane-16: interne `vector_*#`-Aufrufe (z.B. aus `to_bytes_le`) auf den
        // zugehoerigen Overload normalisieren, bevor die Legacy-`#`-Suche laeuft.
        if let Some(res) = crate::vector_prims::resolve_app(name_text, arg_count) {
            if scope.get(res.overload).is_none() {
                if let Some(candidates) = self
                    .program
                    .overload_candidates(res.overload, self.position)?
                {
                    return Ok(candidates.into_iter().map(Type::root).collect());
                }
                if !res.bitfield_fallback {
                    return Err(TypeError::unbound_var(name.span, res.overload.to_owned()));
                }
                // Access/Update ohne Overload: Legacy-`#`-Pfad weiterlaufen
                // lassen; der Bitfield-Fallback dockt dort in spaeteren Lanes an.
            }
        }
        if scope.get(name_text).is_none() {
            if let Some(candidates) = self.program.overload_candidates(name_text, self.position)? {
                return Ok(candidates.into_iter().map(Type::root).collect());
            }
        }
        Ok(vec![self.resolve(&self.id(name, scope)?)?])
    }
    /// Whether a callee formal is an omittable `implicit(..)` argument.
    /// Reads the raw head, bypassing the `implicit`→`atom` display
    /// mapping in `Type::name`.
    fn is_implicit_formal(callee: &Type, formal: &Typ) -> bool {
        match &formal.kind {
            TypKind::App { name, .. } => {
                callee.root.interner.resolve(name.symbol) == Some("implicit")
            }
            _ => false,
        }
    }

    /// Align call-site actuals against callee formals, skipping omitted
    /// `implicit(..)` formals (OCaml `infer_funapp`, e.g.
    /// `extzv([bitzero])` for `(implicit('m), bits('n))`). Returns
    /// per-formal the actual index, or `None` for an omitted implicit.
    /// Passing every formal explicitly (equal counts) keeps working;
    /// anything else is a hard arity error (no ambiguous partial
    /// omission).
    fn align_call(
        callee: &Type,
        formal: &[Typ],
        actual_count: usize,
        span: Span,
    ) -> TypeResult<Vec<Option<usize>>> {
        if actual_count == formal.len() {
            return Ok((0..formal.len()).map(Some).collect());
        }
        let mut alignment = Vec::with_capacity(formal.len());
        let mut next = 0;
        for formal_typ in formal {
            if Self::is_implicit_formal(callee, formal_typ) {
                alignment.push(None);
            } else {
                if next >= actual_count {
                    return Err(arity(span, formal.len(), actual_count));
                }
                alignment.push(Some(next));
                next += 1;
            }
        }
        if next != actual_count {
            return Err(arity(span, formal.len(), actual_count));
        }
        Ok(alignment)
    }
    /// Infer call operands, retrying failed ones against inferred
    /// siblings. An operand whose result variables nothing pins
    /// (omitted implicits, e.g. `extzv(..)` as an `==` operand) retries
    /// as a call against each inferred sibling type; the first success
    /// wins and checks downstream like any annotation. An unambiguous
    /// result signature also guides unresolved inner arguments, which are
    /// checked before their inferred types are used. Direct nesting stays
    /// within the shared synthesis budget.
    fn infer_app_actuals(
        &mut self,
        args: &[Exp],
        scope: &Scope,
        span: Span,
    ) -> TypeResult<Vec<Type>> {
        let mut actual: Vec<Option<Type>> = Vec::with_capacity(args.len());
        let mut first_error = None;
        for arg in args {
            match self.infer(arg, scope) {
                Ok(typ) => actual.push(Some(typ)),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    actual.push(None);
                }
            }
        }
        let Some(first_error) = first_error else {
            return Ok(actual
                .into_iter()
                .map(|typ| typ.expect("all operands inferred"))
                .collect());
        };
        for index in 0..args.len() {
            if actual[index].is_some() {
                continue;
            }
            let ExpKind::App {
                func: inner,
                args: inner_args,
            } = &args[index].kind
            else {
                return Err(first_error);
            };
            let inner_candidates = self.call_candidates(inner, scope, inner_args.len())?;
            let siblings: Vec<Type> = actual.iter().filter_map(|typ| typ.clone()).collect();
            let inferred = inner_args
                .iter()
                .map(|arg| self.infer(arg, scope))
                .collect::<TypeResult<Vec<_>>>();
            let inner_actual = match inferred {
                Ok(actual) => actual,
                Err(error) => {
                    // A unique signature plus a unique result hint determines
                    // the formal argument types without guessing an overload.
                    if inner_candidates.len() != 1
                        || siblings.len() != 1
                        || self.synth_depth >= SYNTH_DIRECT_LIMIT
                    {
                        return Err(error);
                    }
                    let callee = self.instantiate_relaxed(
                        inner_candidates[0].clone(),
                        &[],
                        Some(&siblings[0]),
                        span,
                    )?;
                    let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                        return Err(unsupported(span, "application of non-function"));
                    };
                    let alignment = Self::align_call(&callee, formal, inner_args.len(), span)?;
                    let Some(context) = self.context.clone() else {
                        return Err(error);
                    };
                    let returning = Type::root(context);
                    if !matches!(returning.node().kind, TypKind::Fn { .. }) {
                        return Err(error);
                    }
                    let returning = returning.child(Step::Result);
                    let scoped = Arc::new(scope.clone());
                    let tasks = alignment
                        .iter()
                        .enumerate()
                        .rev()
                        .filter_map(|(index, slot)| {
                            slot.map(|actual| Task {
                                exp: &inner_args[actual],
                                expected: callee.child(Step::Argument(index)),
                                scope: scoped.clone(),
                                returning: returning.clone(),
                            })
                        })
                        .collect();
                    let previous_scope = self.proof_scope.clone();
                    self.synth_depth += 1;
                    let checked = self.run(tasks);
                    self.synth_depth -= 1;
                    self.proof_scope = previous_scope;
                    checked?;
                    inner_args
                        .iter()
                        .map(|arg| self.infer(arg, scope))
                        .collect::<TypeResult<Vec<_>>>()?
                }
            };
            let mut recovered = None;
            for sibling in &siblings {
                match self.select_call(inner_candidates.clone(), &inner_actual, Some(sibling), span)
                {
                    Ok(callee) => {
                        // Like the `App` arm: the operand denotes the
                        // result, not the callee scheme.
                        let result = self.resolve(&callee.child(Step::Result))?;
                        recovered = Some(result);
                        break;
                    }
                    Err(_) => continue,
                }
            }
            let Some(result) = recovered else {
                return Err(first_error);
            };
            actual[index] = Some(self.value_type(&args[index], result));
        }
        Ok(actual
            .into_iter()
            .map(|typ| typ.expect("all operands recovered"))
            .collect())
    }
    fn select_call(
        &self,
        candidates: Vec<Type>,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
    ) -> TypeResult<Type> {
        let single = candidates.len() == 1;
        let mut last = None;
        for callee in candidates {
            let original = callee.root.clone();
            let attempt = (|| {
                let TypKind::Fn { args, .. } = &callee.node().kind else {
                    return Err(unsupported(span,"overload candidate is not an elaborated function (mapping expansion may be required)"));
                };
                let alignment = Self::align_call(&callee, args, actual.len(), span)?;
                let explicit: Vec<usize> = alignment
                    .iter()
                    .enumerate()
                    .filter_map(|(formal, slot)| slot.map(|_| formal))
                    .collect();
                let callee = if callee.root.typschm.quant.is_empty() {
                    callee
                } else {
                    self.instantiate(callee, actual, expected, span)?
                };
                for (actual_index, typ) in actual.iter().enumerate() {
                    // Actual values may be opened, but a formal existential
                    // is an obligation whose predicate must remain intact.
                    if matches!(typ.node().kind, TypKind::Unknown) {
                        continue;
                    }
                    let formal = callee.child(Step::Argument(explicit[actual_index]));
                    let found = self.open_existential(typ);
                    self.compatible(&formal, &found, span)?;
                }
                if let Some(expected) = expected {
                    let result = self.open_existential(&callee.child(Step::Result));
                    self.compatible(expected, &result, span)?;
                }
                Ok(callee)
            })();
            match attempt {
                Ok(callee) => {
                    *self.picked.borrow_mut() = Some(original);
                    return Ok(callee);
                }
                // OCaml tries every overload candidate: a gap in one
                // (`Unsupported`) must not abort the others; if none
                // matches, the last error (of any kind) is reported.
                Err(error) => last = Some(error),
            }
        }
        if single {
            return Err(last.expect("one attempted candidate"));
        }
        Err(TypeError {
            span,
            kind: TypeErrorKind::QuantError(format!(
                "no matching overload for `{}`{}",
                self.call_hint.borrow(),
                last.map(|e| format!(": {e}")).unwrap_or_default()
            )),
        })
    }
    fn literal(&mut self, lit: &Lit) -> TypeResult<Type> {
        Ok(match crate::canonical_literal::classify(lit, &self.it)? {
            crate::canonical_literal::LiteralType::Primitive(name) => self.builtin(name),
            crate::canonical_literal::LiteralType::Refined(name, arg) => self.refined(name, arg),
        })
    }

    /// Refined singleton type (`atom(0)`, `bitvector(2)`, ...). Constant
    /// shapes are shared per checker: rebuilding an identical Scheme and
    /// interner per literal occurrence is what retains tens of megabytes in
    /// deep programs. Non-constant arguments still get a fresh type.
    fn refined(&mut self, name: &str, arg: TypArgKind) -> Type {
        let key = match &arg {
            TypArgKind::NExp(node) => match &node.kind {
                NExpKind::Constant(value) => Some((name.to_owned(), format!("nexp:{value}"))),
                _ => None,
            },
            TypArgKind::Bool(node) => match &node.kind {
                ConstraintKind::True => Some((name.to_owned(), "bool:true".to_owned())),
                ConstraintKind::False => Some((name.to_owned(), "bool:false".to_owned())),
                _ => None,
            },
            _ => None,
        };
        if let Some(key) = key {
            if let Some(typ) = self.refined_cache.get(&key) {
                return typ.clone();
            }
            let typ = self.fresh_refined(name, arg);
            self.refined_cache.insert(key, typ.clone());
            return typ;
        }
        self.fresh_refined(name, arg)
    }

    fn fresh_refined(&self, name: &str, arg: TypArgKind) -> Type {
        let mut it = SimpleInterner::new();
        let name = Name::dummy(it.intern(name));
        Type::owned(
            Node::dummy(TypKind::App {
                name,
                args: vec![Node::dummy(arg)],
            }),
            Arc::new(it),
        )
    }
    fn vector_type(&self, length: usize, element: Type) -> Type {
        let mut interner = SimpleInterner::new();
        let name = Name::dummy(interner.intern("vector"));
        let variable = interner.intern("__element");
        let mut typ = Type::owned(
            Node::dummy(TypKind::App {
                name,
                args: vec![
                    Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Constant(
                        length.to_string(),
                    )))),
                    Node::dummy(TypArgKind::Typ(Node::dummy(TypKind::Var(variable)))),
                ],
            }),
            Arc::new(interner),
        );
        typ.substitutions = Arc::new(Substitutions {
            types: vec![(variable, element)],
            scalars: Vec::new(),
        });
        typ
    }
    /// Composite types borrow their elements; no recursive AST clone is needed.
    /// Elements are moved into the substitution map, never cloned per read.
    fn tuple_type(&self, elements: Vec<Type>) -> Type {
        let mut interner = SimpleInterner::new();
        let mut types = Vec::new();
        let items = elements
            .into_iter()
            .enumerate()
            .map(|(i, element)| {
                let kid = interner.intern(&format!("__element_{i}"));
                types.push((kid, element));
                Node::dummy(TypKind::Var(kid))
            })
            .collect();
        let mut typ = Type::owned(Node::dummy(TypKind::Tuple(items)), Arc::new(interner));
        typ.substitutions = Arc::new(Substitutions {
            types,
            scalars: Vec::new(),
        });
        typ
    }
    /// A var declaration may bind fresh identifiers, including tuple leaves,
    /// but cannot shadow existing locals, enum members or registers.
    /// Mirrors OCaml `type_check.ml::bind_lexp` freshness checks.
    /// OCaml `bind_assignment` in `check_block`: an assignment whose target
    /// names no local or register declares a mutable variable for the rest
    /// of the block. Returns the declared type and the extended scope.
    /// OCaml `exp_unconditionally_returns` / if-throw: the branch never
    /// continues, so the rest of the block assumes the negated condition.
    fn branch_escapes(exp: &Exp) -> bool {
        match &exp.kind {
            ExpKind::Return(_) | ExpKind::Throw(_) => true,
            ExpKind::Block(items) => items.last().is_some_and(|last| {
                matches!(last.kind, ExpKind::Return(_))
                    || (items.len() == 1 && matches!(last.kind, ExpKind::Throw(_)))
            }),
            _ => false,
        }
    }
    fn declaring_assignment(
        &mut self,
        assign: &Exp,
        lexp: &LExp,
        value: &Exp,
        scope: &Arc<Scope>,
    ) -> TypeResult<Option<(Type, Arc<Scope>)>> {
        let mut declares = false;
        let mut work = vec![lexp];
        while let Some(current) = work.pop() {
            match &current.kind {
                LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
                    let text = self.name(name)?;
                    declares |= scope.get(text).is_none()
                        && self.program.register(text, self.position).is_none();
                }
                LExpKind::Tuple(items) => work.extend(items.iter()),
                _ => return Ok(None),
            }
        }
        if !declares {
            return Ok(None);
        }
        self.record_intro(assign);
        let bound = if let LExpKind::Typ { typ, .. } = &lexp.kind {
            self.annotation(typ)?
        } else {
            self.infer(value, scope)?;
            (*self
                .inferred
                .get(&(value as *const Exp))
                .expect("inferred initializer")
                .declared)
                .clone()
        };
        let child = self.mutable_scope(lexp, bound.clone(), scope)?;
        if self.recorder.is_some() {
            self.record_lexp(lexp, &bound);
        }
        Ok(Some((bound, Arc::new(child))))
    }
    /// Getter/updater of a bitfield accessor (`_get_T_f`, `_update_T_f`) for a
    /// value of record-like type `base`, if one is declared.
    fn bitfield_accessor(&self, prefix: &str, base: &Type, field: &str) -> TypeResult<Option<Type>> {
        let Some(typ_name) = base.name() else {
            return Ok(None);
        };
        if !matches!(base.node().kind, TypKind::Id(_)) {
            return Ok(None);
        }
        let id = format!("{prefix}_{typ_name}_{field}");
        match self.program.value(&id, self.position) {
            Some(scheme) => Ok(Some(self.resolve(&Type::root(scheme.clone()))?)),
            None => Ok(None),
        }
    }
    /// OCaml `infer_exp` fallback for `v[FIELD]` / `[v with FIELD = e]` on a
    /// bitfield: a call of the generated accessor. Returns the selected callee
    /// and the indices of the source arguments it consumes.
    fn bitfield_call(
        &mut self,
        exp: &Exp,
        scope: &Scope,
        expected: Option<&Type>,
    ) -> TypeResult<Option<(Type, Vec<usize>)>> {
        let ExpKind::App { func, args } = &exp.kind else {
            return Ok(None);
        };
        let (prefix, arity) = match self.name(func)? {
            "vector_access#" => ("_get", 2),
            "vector_update#" => ("_update", 3),
            _ => return Ok(None),
        };
        if args.len() != arity {
            return Ok(None);
        }
        let ExpKind::Id(field) = &args[1].kind else {
            return Ok(None);
        };
        let field_text = self.name(field)?.to_owned();
        let Ok(base) = self.infer(&args[0], scope) else {
            return Ok(None);
        };
        let base = self.resolve(&base)?;
        let Some(accessor) = self.bitfield_accessor(prefix, &base, &field_text)? else {
            return Ok(None);
        };
        let mut actual = vec![base];
        let mut kept = vec![0];
        if arity == 3 {
            actual.push(self.infer(&args[2], scope)?);
            kept.push(2);
        }
        let callee = self.select_call(vec![accessor], &actual, expected, exp.span)?;
        self.calls.insert(exp as *const Exp, callee.clone());
        Ok(Some((callee, kept)))
    }
    fn mutable_scope(&self, lexp: &LExp, typ: Type, parent: &Arc<Scope>) -> TypeResult<Scope> {
        let mut child = Scope {
            parent: Some(parent.clone()),
            locals: Vec::new(),
            facts: Vec::new(),
        };
        let mut work = vec![(lexp, typ)];
        while let Some((lexp, typ)) = work.pop() {
            let typ = self.resolve(&typ)?;
            match &lexp.kind {
                LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
                    let name_text = self.name(name)?;
                    if parent.get(name_text).is_some()
                        || self.program.register(name_text, self.position).is_some()
                        || self
                            .program
                            .enumeration_member(name_text, self.position)
                            .is_some()
                        || child.locals.iter().any(|(bound, _)| bound == name_text)
                    {
                        return Err(unsupported(
                            lexp.span,
                            "var must declare a fresh mutable variable",
                        ));
                    }
                    let declared = if let LExpKind::Typ {
                        typ: annotation, ..
                    } = &lexp.kind
                    {
                        let annotated = self.annotation(annotation)?;
                        self.compatible(&annotated, &typ, lexp.span)?;
                        annotated
                    } else {
                        typ
                    };
                    // Unlike an immutable binding, a mutable plain scalar has
                    // no stable singleton witness shared by subsequent reads.
                    // Each read opens its own rigid witness via value_type.
                    child.locals.push((name_text.to_owned(), (declared, true)));
                }
                LExpKind::Tuple(items) => {
                    let TypKind::Tuple(types) = &typ.node().kind else {
                        return Err(unsupported(
                            lexp.span,
                            "mutable tuple declaration requires a tuple type",
                        ));
                    };
                    if items.len() != types.len() {
                        return Err(arity(lexp.span, types.len(), items.len()));
                    }
                    for (i, item) in items.iter().enumerate().rev() {
                        work.push((item, typ.child(Step::Tuple(i))));
                    }
                }
                _ => {
                    return Err(unsupported(
                        lexp.span,
                        "var declaration requires fresh identifier or tuple targets",
                    ))
                }
            }
        }
        Ok(child)
    }
    /// Prove a numeric mini-obligation with view-linked variables (B2).
    /// Mirrors `scalar_equal`: only a valid `program.prove` verdict is
    /// accepted, anything else is `ConstraintNotProved` (`Unknown` never
    /// becomes a proof). Emitter rejections (symbolic powers, numeric
    /// applications, unexpandable synonyms) surface as `Unsupported`.
    /// A syntactically decided obligation short-circuits without a query.
    ///
    /// Variable linking: every distinct mini variable resolves to a real
    /// view — nearest scope local (witness NExp + local view) first, else
    /// the function-scheme view (fresh node, text rigid). Unresolvable
    /// variables are `Unsupported`, never guessed. Names coincide with
    /// assumption-side names only on true identity (scope-Arc equality),
    /// so a missed link can fail closed but never prove wrong.
    fn prove_vector_bounds(
        &self,
        obligation: &crate::nexp::NConstraint,
        scope: &Scope,
        span: Span,
        what: &str,
    ) -> TypeResult<()> {
        if let Some(true) = obligation.simplify().as_bool() {
            return Ok(());
        }
        let mut vars = Vec::new();
        obligation.free_vars(&mut vars);
        vars.sort();
        vars.dedup();
        let context_view = self.context.as_ref().map(|s| Type::root(s.clone()));
        let scope_locals = scope;
        let mut git = SimpleInterner::new();
        // Phase 1: alle Symbole internieren (danach keine Mutation mehr).
        let syms: Vec<Symbol> = vars.iter().map(|text| git.intern(text)).collect();
        // Phase 2: frische Knoten für Fallback-Variablen bauen.
        enum Slot {
            Local,
            Witness(Scalar),
            Fresh(usize),
        }
        let mut fresh_args: Vec<sail_ast::TypArg> = Vec::new();
        let mut slots = Vec::new();
        for (text, sym) in vars.iter().zip(syms.iter()) {
            let witness = self
                .witness_links
                .borrow()
                .iter()
                .find(|(name, _)| name == text)
                .map(|(_, scalar)| scalar.clone());
            if scope.get(text).is_some() {
                slots.push((text, sym, Slot::Local));
            } else if let Some(scalar) = witness {
                slots.push((text, sym, Slot::Witness(scalar)));
            } else {
                if context_view.is_none() {
                    return Err(unsupported(
                        span,
                        format!("index variable {text} is unbound"),
                    ));
                }
                fresh_args.push(Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(
                    *sym,
                )))));
                let idx = fresh_args.len() - 1;
                slots.push((text, sym, Slot::Fresh(idx)));
            }
        }
        // Phase 3: Goal-Constraint bauen (letzte Mutation).
        let constraint = crate::conv::constraint_to_ast(obligation, &mut git);
        // Phase 4: Map (nur noch geteilte Ausleihen).
        let mut map = Vec::new();
        for (text, sym, slot) in &slots {
            let (arg, ctx) = match slot {
                Slot::Local => {
                    let (typ, _) = scope.get(text).expect("slot decided");
                    // Witness-NExp aus Local-Sicht (App mit genau einem
                    // NExp-Argument); sonst ehrlich `Unsupported`.
                    let node = match &typ.node().kind {
                        TypKind::App { args, .. } if args.len() == 1 => &args[0],
                        _ => {
                            return Err(unsupported(
                                span,
                                format!("index variable {text} has no numeric witness"),
                            ));
                        }
                    };
                    let TypArgKind::NExp(_) = &node.kind else {
                        return Err(unsupported(
                            span,
                            format!("index variable {text} has no numeric witness"),
                        ));
                    };
                    (node, typ.proof_context())
                }
                Slot::Witness(scalar) => (scalar.argument(), scalar.owner.proof_context()),
                Slot::Fresh(idx) => {
                    // Funktions-Scope: frischer Knoten, Text-Rigid über die
                    // Funktions-Sicht (derselbe Arc wie in den Annahmen).
                    let node = &fresh_args[*idx];
                    (
                        node,
                        Context {
                            interner: &git,
                            variables: &EMPTY_PROOF_VARS,
                            bindings: Some(context_view.as_ref().expect("checked")),
                        },
                    )
                }
            };
            map.push((
                **sym,
                Argument {
                    argument: arg,
                    context: ctx,
                },
            ));
        }
        let goalmap = GoalMap { map };
        let view = View {
            constraint: &constraint,
            context: Context {
                interner: &git,
                variables: &EMPTY_PROOF_VARS,
                bindings: Some(&goalmap),
            },
            negated: false,
        };
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let local_types: Vec<&Type> = vars
            .iter()
            .filter_map(|text| scope_locals.get(text).map(|(typ, _)| typ))
            .collect();
        let mut assumptions = self.proof_assumptions(&scope);
        assumptions.extend(self.marker_assumptions(&local_types));
        assumptions.extend(self.marker_assumptions(&self.fact_owners()));
        let query = CanonicalQuery::implication(&assumptions, view, None)
            .map_err(|e| unsupported(span, format!("{what} [{obligation}]: {e}")))?;
        if !self.program.prove(query).is_valid() {
            return Err(TypeError::constraint_not_proved(
                span,
                format!("{what}: {obligation}"),
            ));
        }
        Ok(())
    }
    /// Length argument of a resolved vector/bitvector root, following the
    /// substitution chain (`Scalar::resolved`).
    ///
    /// The raw AST node still carries the unsubstituted variable (e.g. `'n`
    /// from `bits('n)`); call-result bindings may already close it to a
    /// compound length (`8 * 'n` from `sail_zeros(8 * n)`). Using the raw
    /// node would state the wrong obligation (`< 'n` instead of `< 8 * 'n`),
    /// so the followed node is always used. Name resolution for the
    /// followed node must see the bound owner's tables (symbols collide by
    /// number across tables), hence the union includes the grounded
    /// owner's interners first.
    fn vector_len_nexp(&self, root: &Type, span: Span) -> TypeResult<crate::nexp::Nexp> {
        let TypKind::App { args, .. } = &root.node().kind else {
            return Err(unsupported(span, "Invalid vector assignment l-expression"));
        };
        args.first()
            .ok_or_else(|| unsupported(span, "Invalid vector assignment l-expression"))?;
        let top = Scalar {
            owner: root.clone(),
            index: 0,
        };
        self.ground_scalar(&top, &[root.clone()], &mut Vec::new())
            .ok_or_else(|| unsupported(span, "Invalid vector assignment l-expression"))
    }

    /// Find the binding for `symbol` in the home owner's own substitution
    /// map. Same-table only: cross-table symbols collide by number, and
    /// even texts collide across scopes (function `'n` vs instantiated
    /// `'n`), so consulting a foreign map can alias distinct rigids.
    /// A variable occurrence is bound by its own owner's map; anything
    /// else stays rigid and links later via scope/function views.
    fn find_binding(symbol: Symbol, home: &Type) -> Option<Scalar> {
        home.substitutions.get_scalar(symbol).cloned()
    }

    /// Follow a scalar argument through substitution chains and lower it
    /// to a Mini-`Nexp`, grounding variables bound (transitively) to
    /// constants. Unbound variables stay rigid and link later via
    /// scope/function views (`prove_vector_bounds`). Cycle-guarded
    /// (`visited` over substitution-`Arc` + symbol) and step-capped;
    /// `None` is honest `Unsupported`, never a guess. The top chain is
    /// an explicit loop; node lowering goes through `ground_node`.
    fn ground_scalar(
        &self,
        scalar: &Scalar,
        extra: &[Type],
        visited: &mut Vec<(usize, Symbol)>,
    ) -> Option<crate::nexp::Nexp> {
        let mut owners: Vec<Type> = Vec::with_capacity(extra.len() + 1);
        owners.push(scalar.owner.clone());
        owners.extend(extra.iter().cloned());
        let mut current = scalar.clone();
        let mut fuel = 64u32;
        loop {
            if fuel == 0 {
                return None;
            }
            fuel -= 1;
            let symbol = match &current.argument().kind {
                TypArgKind::NExp(node) => match &node.kind {
                    NExpKind::Var(symbol) => *symbol,
                    _ => break,
                },
                TypArgKind::Bool(constraint) => match &constraint.kind {
                    ConstraintKind::Var(symbol) => *symbol,
                    _ => break,
                },
                _ => break,
            };
            if !owners
                .iter()
                .any(|owner| Arc::ptr_eq(&owner.substitutions, &current.owner.substitutions))
            {
                owners.insert(0, current.owner.clone());
            }
            let key = (Arc::as_ptr(&current.owner.substitutions) as usize, symbol);
            if visited.contains(&key) {
                break;
            }
            let Some(bound) = Self::find_binding(symbol, &current.owner) else {
                break;
            };
            visited.push(key);
            current = bound;
        }
        match &current.argument().kind {
            TypArgKind::NExp(node) => {
                self.ground_node(node.clone(), current.owner.clone(), owners, visited)
            }
            _ => None,
        }
    }

    /// Rigid variable of an opened witness (not a function quantifier): a
    /// uniquely named Mini variable linked back to its binder.
    fn witness_link(&self, owner: &Type, symbol: Symbol) -> Option<crate::nexp::Nexp> {
        let name = owner.root.interner.resolve(symbol)?;
        if (owner as &dyn Bindings).rigid(name).is_none() {
            return None;
        }
        if self
            .context
            .as_ref()
            .is_some_and(|context| Arc::ptr_eq(context, &owner.scope))
        {
            return None;
        }
        let mut it = SimpleInterner::new();
        let variable = it.intern(name);
        let atom = Name::dummy(it.intern("atom"));
        let typ = Node::dummy(TypKind::App {
            name: atom,
            args: vec![Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(variable))))],
        });
        let synthetic = Type {
            root: Arc::new(Scheme {
                typschm: TypSchm {
                    quant: Vec::new(),
                    typ,
                },
                interner: Arc::new(it),
            }),
            path: Vec::new(),
            scope: owner.scope.clone(),
            substitutions: Arc::default(),
            implicit_argument: false,
        };
        let mut links = self.witness_links.borrow_mut();
        let unique = format!("{name}#{}", links.len());
        links.push((unique.clone(), Scalar { owner: synthetic, index: 0 }));
        Some(crate::nexp::Nexp::Var(unique))
    }
    /// Resolve a rigid remainder to Mini text (home tables first, then the
    /// checker interner). `None` when unresolvable (fail-closed).
    fn rigid_text(
        owners: &[Type],
        it: &SimpleInterner,
        symbol: Symbol,
    ) -> Option<crate::nexp::Nexp> {
        let mut chain: Vec<&dyn sail_ast::Interner> = Vec::with_capacity(2 * owners.len() + 1);
        for table in owners {
            chain.push(&*table.root.interner as &dyn sail_ast::Interner);
            chain.push(&*table.scope.interner as &dyn sail_ast::Interner);
        }
        chain.push(it as &dyn sail_ast::Interner);
        let union = crate::vector_lexp::Union::over(chain);
        union
            .resolve(symbol)
            .map(|text| crate::nexp::Nexp::Var(text.to_owned()))
    }

    /// Iterative Mini lowering over owned nodes (no recursion): an
    /// explicit stack processes followed chains (inside `Build`), node
    /// shapes (`Build`) and combinations (`Combine`); results flow
    /// through a value stack that stays balanced (every task pushes
    /// exactly one value). Variable chains follow same-table bindings
    /// with the shared `visited` cycle guard; unbound variables resolve
    /// to rigid text; total steps are capped (fail-closed). Behavior
    /// matches the former mutually-recursive pair exactly.
    fn ground_node(
        &self,
        node: NExp,
        owner: Type,
        owners: Vec<Type>,
        visited: &mut Vec<(usize, Symbol)>,
    ) -> Option<crate::nexp::Nexp> {
        use crate::nexp::Nexp as Mini;
        // Combine ops: 0 = Add, 1 = Sub, 2 = Mul, 3 = Neg, 4 = Exp.
        enum Task {
            Build {
                node: NExp,
                owner: Type,
                owners: Vec<Type>,
                chain: Vec<(usize, Symbol)>,
            },
            Combine {
                op: u8,
            },
        }
        let mut stack = vec![Task::Build {
            node,
            owner,
            owners,
            chain: visited.clone(),
        }];
        let mut values: Vec<Option<Mini>> = Vec::new();
        let mut steps = 0u32;
        // A completed task always leaves exactly one value behind.
        enum Terminal {
            Combined,
            Value(Option<Mini>),
        }
        while let Some(task) = stack.pop() {
            steps += 1;
            if steps > 10_000 {
                return None;
            }
            match task {
                Task::Build {
                    mut node,
                    mut owner,
                    mut owners,
                    mut chain,
                } => {
                    // Follow phase: resolve variables through same-table
                    // bindings (cycle-guarded); ends at a rigid variable,
                    // a constant, a compound shape (pushed below), or an
                    // unsupported shape.
                    let terminal: Terminal = loop {
                        match node.kind {
                            NExpKind::Var(symbol) => {
                                if !owners.iter().any(|known| {
                                    Arc::ptr_eq(&known.substitutions, &owner.substitutions)
                                }) {
                                    owners.insert(0, owner.clone());
                                }
                                let key = (Arc::as_ptr(&owner.substitutions) as usize, symbol);
                                if chain.contains(&key) {
                                    break Terminal::Value(
                                        self.witness_link(&owner, symbol)
                                            .or_else(|| Self::rigid_text(&owners, &self.it, symbol)),
                                    );
                                }
                                let Some(bound) = Self::find_binding(symbol, &owner) else {
                                    break Terminal::Value(
                                        self.witness_link(&owner, symbol)
                                            .or_else(|| Self::rigid_text(&owners, &self.it, symbol)),
                                    );
                                };
                                chain.push(key);
                                match &bound.argument().kind {
                                    TypArgKind::NExp(next) => {
                                        node = next.clone();
                                        owner = bound.owner.clone();
                                        continue;
                                    }
                                    _ => break Terminal::Value(None),
                                }
                            }
                            NExpKind::Constant(ref text) => {
                                break Terminal::Value(text.parse::<i128>().ok().map(Mini::Const));
                            }
                            NExpKind::Sum(..) | NExpKind::Minus(..) | NExpKind::Times(..) => {
                                let op = match node.kind {
                                    NExpKind::Sum(..) => 0,
                                    NExpKind::Minus(..) => 1,
                                    _ => 2,
                                };
                                let (first, second) = match node.kind {
                                    NExpKind::Sum(first, second)
                                    | NExpKind::Minus(first, second)
                                    | NExpKind::Times(first, second) => (first, second),
                                    _ => unreachable!("ground combine shape"),
                                };
                                stack.push(Task::Combine { op });
                                stack.push(Task::Build {
                                    node: (*second).clone(),
                                    owner: owner.clone(),
                                    owners: owners.clone(),
                                    chain: chain.clone(),
                                });
                                stack.push(Task::Build {
                                    node: (*first).clone(),
                                    owner,
                                    owners,
                                    chain,
                                });
                                break Terminal::Combined;
                            }
                            NExpKind::App { ref func, ref args }
                                if args.len() == 2
                                    && matches!(
                                        owner.root.interner.resolve(func.symbol),
                                        Some("div" | "mod")
                                    ) =>
                            {
                                let op = if owner.root.interner.resolve(func.symbol) == Some("div") { 5 } else { 6 };
                                let (first, second) = (args[0].clone(), args[1].clone());
                                stack.push(Task::Combine { op });
                                stack.push(Task::Build {
                                    node: second,
                                    owner: owner.clone(),
                                    owners: owners.clone(),
                                    chain: chain.clone(),
                                });
                                stack.push(Task::Build {
                                    node: first,
                                    owner,
                                    owners,
                                    chain,
                                });
                                break Terminal::Combined;
                            }
                            NExpKind::App { ref func, ref args }
                                if args.len() == 1 && owner.root.interner.resolve(func.symbol) == Some("abs") =>
                            {
                                stack.push(Task::Combine { op: 7 });
                                stack.push(Task::Build {
                                    node: args[0].clone(),
                                    owner,
                                    owners,
                                    chain,
                                });
                                break Terminal::Combined;
                            }
                            NExpKind::Neg(..) | NExpKind::Exp(..) => {
                                let (op, inner) = match node.kind {
                                    NExpKind::Neg(inner) => (3, inner),
                                    NExpKind::Exp(inner) => (4, inner),
                                    _ => unreachable!("ground combine shape"),
                                };
                                stack.push(Task::Combine { op });
                                stack.push(Task::Build {
                                    node: (*inner).clone(),
                                    owner,
                                    owners,
                                    chain,
                                });
                                break Terminal::Combined;
                            }
                            _ => break Terminal::Value(None),
                        }
                    };
                    if let Terminal::Value(value) = terminal {
                        values.push(value);
                    }
                }
                Task::Combine { op } => {
                    // Binary ops pop right, then left (left was pushed last).
                    let result = if op <= 2 || op == 5 || op == 6 {
                        match (values.pop(), values.pop()) {
                            (Some(right), Some(left)) => {
                                let (right, left) = (right?, left?);
                                Some(match op {
                                    0 => Mini::Add(Box::new(left), Box::new(right)),
                                    1 => Mini::Sub(Box::new(left), Box::new(right)),
                                    5 => Mini::Div(Box::new(left), Box::new(right)),
                                    6 => Mini::Mod(Box::new(left), Box::new(right)),
                                    _ => Mini::Mul(Box::new(left), Box::new(right)),
                                })
                            }
                            _ => None,
                        }
                    } else {
                        match values.pop() {
                            Some(Some(child)) => Some(match op {
                                3 => Mini::Neg(Box::new(child)),
                                7 => Mini::Abs(Box::new(child)),
                                _ => Mini::Exp(Box::new(child)),
                            }),
                            _ => None,
                        }
                    };
                    values.push(result);
                }
            }
        }
        values.pop().flatten()
    }
    /// Appended vector type for `@` (OCaml `E_vector_append`), shared by
    /// checking (`run`) and synthesis (`synthesize`).
    ///
    /// Both sides arrive resolved to the same vector flavor; the result
    /// concatenates the grounded lengths (`bits(n) ++ bits(m) =
    /// bits(n + m)`). Lengths follow the full substitution chain
    /// (`vector_len_nexp`); the result root carries the summed shape with
    /// every variable linked to a side view (same-table precise, never
    /// guessed). Flavor mismatch is hard `Mismatch`; shapeless operands
    /// stay honest `Unsupported`. Like OCaml (`Function append not
    /// found`), `@` needs an `append` overload in scope.
    fn append_result(&self, left_typ: &Type, right_typ: &Type, span: Span) -> TypeResult<Type> {
        // Only the declaration counts; members may dangle
        // (backend builtins like `__id`).
        if !self.program.has_overload("append", self.position) {
            return Err(TypeError::unbound_var(span, "append".to_owned()));
        }
        let mut lunion = crate::vector_lexp::Union::over(vec![
            &*left_typ.root.interner as &dyn sail_ast::Interner,
            &*left_typ.scope.interner as &dyn sail_ast::Interner,
            &*self.it as &dyn sail_ast::Interner,
        ]);
        let mut runion = crate::vector_lexp::Union::over(vec![
            &*right_typ.root.interner as &dyn sail_ast::Interner,
            &*right_typ.scope.interner as &dyn sail_ast::Interner,
            &*self.it as &dyn sail_ast::Interner,
        ]);
        let left_root = crate::vector_lexp_bridge::vector_root_of_canonical(
            left_typ.node(),
            &mut lunion,
            span,
        )?;
        let right_root = crate::vector_lexp_bridge::vector_root_of_canonical(
            right_typ.node(),
            &mut runion,
            span,
        )?;
        // NLL beendet beide Union-Ausleihen vor `&mut`-Gebrauch.
        let len_left = self.vector_len_nexp(&left_typ, span)?;
        let len_right = self.vector_len_nexp(&right_typ, span)?;
        let elem_mini = match (&left_root, &right_root) {
            (
                crate::lexp_bounds::VectorRoot::Bitvector { .. },
                crate::lexp_bounds::VectorRoot::Bitvector { .. },
            ) => None,
            (
                crate::lexp_bounds::VectorRoot::Vector { .. },
                crate::lexp_bounds::VectorRoot::Vector { .. },
            ) => {
                let elem = |typ: &Type| -> TypeResult<crate::nexp::MiniTyp> {
                    let TypKind::App { args, .. } = &typ.node().kind else {
                        return Err(unsupported(span, "append of non-vector type"));
                    };
                    let Some(arg) = args.get(1) else {
                        return Err(unsupported(span, "append of non-vector type"));
                    };
                    let TypArgKind::Typ(node) = &arg.kind else {
                        return Err(unsupported(span, "append of non-vector type"));
                    };
                    let mut union = crate::vector_lexp::Union::over(vec![
                        &*typ.root.interner as &dyn sail_ast::Interner,
                        &*typ.scope.interner as &dyn sail_ast::Interner,
                        &*self.it as &dyn sail_ast::Interner,
                    ]);
                    crate::vector_lexp_bridge::mini_typ_of_canonical(node, &mut union, span)
                };
                let elem_left = elem(&left_typ)?;
                let elem_right = elem(&right_typ)?;
                if elem_left != elem_right {
                    return Err(mismatch(span, &left_typ, &right_typ));
                }
                Some(elem_left)
            }
            _ => {
                return Err(TypeError::mismatch(
                    span,
                    "bitvector".to_owned(),
                    "vector".to_owned(),
                ));
            }
        };
        // Lengths link as whole scalars (`__l`, `__r`): grounding a compound
        // length to its variables and linking each variable to the entire
        // side would substitute the compound expression twice.
        let constant_sum = match (len_left.as_const(), len_right.as_const()) {
            (Some(left), Some(right)) => Some(left + right),
            _ => None,
        };
        let sum = match constant_sum {
            Some(constant) => crate::nexp::Nexp::Const(constant),
            None => crate::nexp::Nexp::Add(
                Box::new(crate::nexp::Nexp::Var("__l".to_owned())),
                Box::new(crate::nexp::Nexp::Var("__r".to_owned())),
            ),
        };
        let left_len_marker = len_left.as_const().is_none();
        let mut git = SimpleInterner::new();
        fn mini_typ_vars(mini: &crate::nexp::MiniTyp, out: &mut Vec<String>) {
            use crate::nexp::MiniTyp as Mini;
            // Iterative pre-order walk (no recursion).
            let mut todo = vec![mini];
            while let Some(current) = todo.pop() {
                match current {
                    Mini::Int | Mini::Bool | Mini::Bit => {}
                    Mini::Bits(n) => n.free_vars(out),
                    Mini::Range(first, second) => {
                        first.free_vars(out);
                        second.free_vars(out);
                    }
                    Mini::Named { args, .. } => {
                        for arg in args {
                            arg.free_vars(out);
                        }
                    }
                    Mini::Fun(first, second) => {
                        todo.push(first);
                        todo.push(second);
                    }
                    Mini::Tuple(items) => {
                        for item in items {
                            todo.push(item);
                        }
                    }
                }
            }
        }
        let _ = left_len_marker;
        let left_texts = vec!["__l".to_owned()];
        let right_texts = vec!["__r".to_owned()];
        // Element-Variablen (Vektor-Flavor) verlinken auf die
        // Element-Sicht (Argument 1) der Seite, die sie traegt.
        let mut left_elem_texts = Vec::new();
        let mut right_elem_texts = Vec::new();
        if let Some(mini) = &elem_mini {
            mini_typ_vars(mini, &mut left_elem_texts);
            // `elem_left == elem_right` ist oben geprueft;
            // beide Seiten tragen dieselben Namen.
            right_elem_texts.clone_from(&left_elem_texts);
        }
        let mut texts = Vec::new();
        sum.free_vars(&mut texts);
        texts.extend(left_elem_texts.iter().cloned());
        texts.sort();
        texts.dedup();
        let mut scalars = Vec::new();
        for text in &texts {
            let (owner, index) = if left_texts.contains(text) {
                (left_typ, 0)
            } else if right_texts.contains(text) {
                (right_typ, 0)
            } else if left_elem_texts.contains(text) {
                (left_typ, 1)
            } else if right_elem_texts.contains(text) {
                (right_typ, 1)
            } else {
                return Err(unsupported(
                    span,
                    format!("append variable {text} is unbound"),
                ));
            };
            scalars.push((
                git.intern(text),
                Scalar {
                    owner: owner.clone(),
                    index,
                }
                .resolved(),
            ));
        }
        let len_node = crate::conv::nexp_to_ast(&sum, &mut git);
        let head = if elem_mini.is_none() {
            "bits"
        } else {
            "vector"
        };
        let head_sym = git.intern(head);
        let mut result_args = vec![Node::dummy(TypArgKind::NExp(len_node))];
        if let Some(mini) = &elem_mini {
            let elem_node = crate::vector_lexp_bridge::ast_of_mini_typ(mini, &mut git);
            result_args.push(Node::dummy(TypArgKind::Typ(elem_node)));
        }
        let typ = Node::dummy(TypKind::App {
            name: Name::dummy(head_sym),
            args: result_args,
        });
        let root = Arc::new(Scheme {
            typschm: TypSchm { quant: vec![], typ },
            interner: Arc::new(git),
        });
        Ok(Type {
            root: root.clone(),
            path: Vec::new(),
            scope: root,
            substitutions: Arc::new(Substitutions {
                types: Vec::new(),
                scalars,
            }),
            implicit_argument: false,
        })
    }
    /// Rehome a source-level `NExp` into a fresh interner by text.
    /// Source symbols live in the checker interner; copying numbers would
    /// orphan them in a fresh table (B2b class). Texts round-trip exactly
    /// (`Constant`/`Var`/arithmetik); anything else (`Id`, `If`,
    /// applications) is `None` (honest `Unsupported`, never guessed).
    fn rehome_nexp(&self, node: &NExp, git: &mut SimpleInterner) -> Option<NExp> {
        // Iterative post-order rebuild (no recursion): children first via
        // an explicit stack, assembled bottom-up keyed by node identity
        // (ASTs are trees, so every node is visited exactly once).
        enum Task<'a> {
            Visit(&'a NExp),
            Build(&'a NExp),
        }
        let mut stack = vec![Task::Visit(node)];
        let mut built: HashMap<*const NExp, NExp> = HashMap::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Visit(current) => {
                    if built.contains_key(&(current as *const NExp)) {
                        continue;
                    }
                    stack.push(Task::Build(current));
                    match &current.kind {
                        NExpKind::Sum(first, second)
                        | NExpKind::Minus(first, second)
                        | NExpKind::Times(first, second) => {
                            stack.push(Task::Visit(first));
                            stack.push(Task::Visit(second));
                        }
                        NExpKind::Neg(inner) | NExpKind::Exp(inner) => {
                            stack.push(Task::Visit(inner));
                        }
                        _ => {}
                    }
                }
                Task::Build(current) => {
                    let child = |built: &HashMap<*const NExp, NExp>, node: &NExp| {
                        built.get(&(node as *const NExp)).cloned()
                    };
                    let kind = match &current.kind {
                        NExpKind::Constant(text) => NExpKind::Constant(text.clone()),
                        NExpKind::Var(symbol) => {
                            NExpKind::Var(git.intern(self.it.resolve(*symbol)?))
                        }
                        NExpKind::Sum(first, second) => NExpKind::Sum(
                            Box::new(child(&built, first)?),
                            Box::new(child(&built, second)?),
                        ),
                        NExpKind::Minus(first, second) => NExpKind::Minus(
                            Box::new(child(&built, first)?),
                            Box::new(child(&built, second)?),
                        ),
                        NExpKind::Times(first, second) => NExpKind::Times(
                            Box::new(child(&built, first)?),
                            Box::new(child(&built, second)?),
                        ),
                        NExpKind::Neg(inner) => NExpKind::Neg(Box::new(child(&built, inner)?)),
                        NExpKind::Exp(inner) => NExpKind::Exp(Box::new(child(&built, inner)?)),
                        _ => return None,
                    };
                    built.insert(current as *const NExp, Node::dummy(kind));
                }
            }
        }
        built.remove(&(node as *const NExp))
    }

    /// `E_sizeof` (OCaml `infer_exp`): a type-level variable in expression
    /// position denotes its numeric value. Synthesizes `atom('n)` with the
    /// rehomed node and the function scheme as binder scope, so the rigid
    /// links to assumption-side names (same scope-`Arc`). Names outside
    /// the function quantifiers stay honest `Unsupported` (no orphan
    /// views). Like OCaml (`No possible overloading for __size`), a
    /// `__size` overload must be in scope (the prelude provides it);
    /// compound shapes stay `Unsupported` (OCaml rewrites those via
    /// `add_atom`/`sub_atom`/… calls, which is elaboration, not checking).
    fn sizeof_type(&self, node: &NExp, span: Span) -> TypeResult<Type> {
        let NExpKind::Var(_) = &node.kind else {
            return Err(unsupported(
                span,
                "sizeof of compound size expression needs elaboration",
            ));
        };
        let local_tyvars: Vec<(String, Scalar)> = self
            .tyvars
            .iter()
            .filter_map(|(symbol, scalar)| {
                self.it.resolve(*symbol).map(|name| (name.to_owned(), scalar.clone()))
            })
            .collect();
        if !self.program.has_overload("__size", self.position) {
            return Err(TypeError::unbound_var(span, "__size".to_owned()));
        };
        let mut git = SimpleInterner::new();
        let rehomed = self
            .rehome_nexp(node, &mut git)
            .ok_or_else(|| unsupported(span, "dependent size expression typing"))?;
        let context = self
            .context
            .clone()
            .ok_or_else(|| unsupported(span, "size expression outside function context"))?;
        // Fail closed upfront: every variable must be rigid in the
        // function scope, otherwise the emitter would later report an
        // unbound rigid proof variable. Collector is an explicit stack.
        let view = Type::root(context.clone());
        let mut vars = Vec::new();
        let mut todo: Vec<&NExp> = vec![&rehomed];
        while let Some(current) = todo.pop() {
            match &current.kind {
                NExpKind::Var(symbol) => vars.push(*symbol),
                NExpKind::Constant(_) => {}
                NExpKind::Sum(first, second)
                | NExpKind::Minus(first, second)
                | NExpKind::Times(first, second) => {
                    todo.push(first);
                    todo.push(second);
                }
                NExpKind::Neg(inner) | NExpKind::Exp(inner) => todo.push(inner),
                _ => {}
            }
        }
        for symbol in vars {
            let text = git
                .resolve(symbol)
                .ok_or_else(|| unsupported(span, "dependent size expression typing"))?;
            if local_tyvars.iter().any(|(name, _)| name == text) {
                continue;
            }
            if (&view as &dyn Bindings).rigid(text).is_none() {
                return Err(unsupported(
                    span,
                    format!("size variable {text} is unbound"),
                ));
            }
        }
        let generated: HashMap<String, Symbol> = local_tyvars
            .iter()
            .map(|(name, _)| (name.clone(), git.intern(name)))
            .collect();
        let atom = git.intern("atom");
        let typ = Node::dummy(TypKind::App {
            name: Name::dummy(atom),
            args: vec![Node::dummy(TypArgKind::NExp(rehomed))],
        });
        Ok(Type {
            root: Arc::new(Scheme {
                typschm: TypSchm { quant: vec![], typ },
                interner: Arc::new(git),
            }),
            path: Vec::new(),
            scope: context,
            substitutions: Arc::new(Substitutions {
                types: Vec::new(),
                scalars: local_tyvars
                    .iter()
                    .filter_map(|(name, scalar)| {
                        generated.get(name.as_str()).map(|symbol| (*symbol, scalar.clone()))
                    })
                    .collect(),
            }),
            implicit_argument: false,
        })
    }
    /// Guard assumption synthesis for conditions (OCaml `E_if`
    /// assumptions and `assert_constraint`): numeric comparisons over
    /// atoms contribute their constraint over the operands' *declared*
    /// type variables (`m < 'n` gives `'m < 'n`), so branch bodies can
    /// discharge callee constraints (`truncate` needs `'m <= 'n`);
    /// conjunctions/disjunctions combine iteratively (same text from
    /// different binder scopes stays `None`); `E_constraint` contributes
    /// its rigid-checked constraint; literals contribute `True`/`False`.
    /// Anything unshaped yields `None`: the condition still checks, it
    /// just assumes nothing — current behavior, never a guess. Only
    /// numeric comparisons and `&`/`|` over them; `bool` operators and
    /// custom forms contribute nothing. Boolean nesting uses an explicit
    /// stack (no recursion); leaves go through `guard_leaf`.
    ///
    /// Links map each variable text to a declared-side scalar view
    /// (`Some`, same-table precise) or to a function-scope rigid
    /// (`None`, resolved via the fact scope).
    fn guard_mini(
        &mut self,
        exp: &Exp,
        scope: &Scope,
        span: Span,
    ) -> TypeResult<Option<(crate::nexp::NConstraint, Vec<(String, Option<Scalar>)>)>> {
        use crate::nexp::NConstraint as MiniCons;
        use std::collections::HashMap;
        type Links = Vec<(String, Option<Scalar>)>;
        type Outcome = Option<(MiniCons, Links)>;
        enum Task<'a> {
            Visit(&'a Exp),
            Combine {
                and: bool,
                left: *const Exp,
                right: *const Exp,
                self_ptr: *const Exp,
            },
        }
        // Same text from different binder scopes must not conflate:
        // fail closed instead of proving about the wrong rigid.
        fn provenance(link: &Option<Scalar>, context: &Option<Arc<Scheme>>) -> Option<usize> {
            match link {
                Some(scalar) => Some(Arc::as_ptr(&scalar.owner.scope) as usize),
                None => context.as_ref().map(|s| Arc::as_ptr(s) as usize),
            }
        }
        let mut stack = vec![Task::Visit(exp)];
        let mut done: HashMap<*const Exp, Outcome> = HashMap::new();
        while let Some(task) = stack.pop() {
            match task {
                Task::Visit(current) => {
                    let key = current as *const Exp;
                    if done.contains_key(&key) {
                        continue;
                    }
                    if let ExpKind::App { func, args } = &current.kind {
                        if args.len() == 2 {
                            let op = self.name(func)?;
                            if matches!(op, "&" | "|" | "and_bool" | "or_bool") {
                                stack.push(Task::Combine {
                                    and: matches!(op, "&" | "and_bool"),
                                    left: &args[0] as *const Exp,
                                    right: &args[1] as *const Exp,
                                    self_ptr: key,
                                });
                                stack.push(Task::Visit(&args[1]));
                                stack.push(Task::Visit(&args[0]));
                                continue;
                            }
                        }
                    }
                    let outcome = self.guard_leaf(current, scope, span)?;
                    done.insert(key, outcome);
                }
                Task::Combine {
                    and,
                    left,
                    right,
                    self_ptr,
                } => {
                    let outcome = match (done.remove(&left), done.remove(&right)) {
                        (
                            Some(Some((left_mini, left_links))),
                            Some(Some((right_mini, right_links))),
                        ) => {
                            let mut links = left_links;
                            let mut conflict = false;
                            for (text, link) in right_links {
                                if let Some(existing) =
                                    links.iter().find(|(bound, _)| bound == &text)
                                {
                                    if provenance(&existing.1, &self.context)
                                        != provenance(&link, &self.context)
                                    {
                                        conflict = true;
                                        break;
                                    }
                                } else {
                                    links.push((text, link));
                                }
                            }
                            if conflict {
                                None
                            } else {
                                let combined = if and {
                                    MiniCons::And(Box::new(left_mini), Box::new(right_mini))
                                } else {
                                    MiniCons::Or(Box::new(left_mini), Box::new(right_mini))
                                };
                                Some((combined, links))
                            }
                        }
                        // A true conjunction implies each known conjunct: an
                        // unknown side may be dropped, but only where the fact is
                        // used positively (assert), never when negated.
                        (Some(Some(known)), Some(None) | None) | (Some(None) | None, Some(Some(known)))
                            if and && self.weak_facts.get() =>
                        {
                            Some(known)
                        }
                        _ => None,
                    };
                    done.insert(self_ptr, outcome);
                }
            }
        }
        Ok(done.remove(&(exp as *const Exp)).flatten())
    }

    /// Leaf facts for `guard_mini` (no boolean nesting, handled by the
    /// driver below): numeric
    /// assumptions and `assert_constraint`): numeric comparisons over
    /// atoms contribute their constraint over the operands' *declared*
    /// type variables (`m < 'n` gives `'m < 'n`), so branch bodies can
    /// discharge callee constraints (`truncate` needs `'m <= 'n`);
    /// conjunctions/disjunctions combine recursively; `E_constraint`
    /// contributes its (rigid-checked) constraint; literals contribute
    /// `True`/`False`. Anything unshaped yields `None`: the condition
    /// still checks, it just assumes nothing — current behavior, never
    /// a guess. Only numeric comparisons and `&`/`|` over them; `bool`
    /// operators and custom forms contribute nothing.
    ///
    /// Links map each variable text to a declared-side scalar view
    /// (`Some`, same-table precise) or to a function-scope rigid
    /// (`None`, resolved via the fact scope).
    fn guard_leaf(
        &mut self,
        exp: &Exp,
        scope: &Scope,
        _span: Span,
    ) -> TypeResult<Option<(crate::nexp::NConstraint, Vec<(String, Option<Scalar>)>)>> {
        use crate::nexp::{NConstraint as MiniCons, Nexp as Mini};
        match &exp.kind {
            ExpKind::App { func, args } if args.len() == 2 => {
                let op = self.name(func)?;
                let ctor: fn(Box<Mini>, Box<Mini>) -> MiniCons = match op {
                    "<" => MiniCons::Lt,
                    "<=" => MiniCons::Le,
                    ">" => MiniCons::Gt,
                    ">=" => MiniCons::Ge,
                    "==" => MiniCons::Eq,
                    "!=" => MiniCons::Neq,
                    _ => return Ok(None),
                };
                let mut sides = Vec::with_capacity(2);
                for arg in args {
                    let inferred = self.infer(arg, scope)?;
                    let value = self.resolve(&inferred)?;
                    let TypKind::App { .. } = &value.node().kind else {
                        return Ok(None);
                    };
                    if value.name() != Some("atom") {
                        return Ok(None);
                    }
                    // Declared (pre-opening) views carry the linkable
                    // variables; opened `__value` witnesses cannot link by
                    // design.
                    let declared = self
                        .inferred
                        .get(&(arg as *const Exp))
                        .map(|inferred| (*inferred.declared).clone())
                        .unwrap_or_else(|| value.clone());
                    let grounded = self.ground_scalar(
                        &Scalar {
                            owner: declared.clone(),
                            index: 0,
                        },
                        &[declared.clone()],
                        &mut Vec::new(),
                    );
                    let Some(grounded) = grounded else {
                        return Ok(None);
                    };
                    sides.push((declared, grounded));
                }
                let [(left_typ, left_nexp), (right_typ, right_nexp)] = sides.as_slice() else {
                    unreachable!("two comparison operands")
                };
                let mut left_texts = Vec::new();
                left_nexp.free_vars(&mut left_texts);
                let mut right_texts = Vec::new();
                right_nexp.free_vars(&mut right_texts);
                let obligation = ctor(Box::new(left_nexp.clone()), Box::new(right_nexp.clone()));
                let mut texts = Vec::new();
                obligation.free_vars(&mut texts);
                texts.sort();
                texts.dedup();
                let mut links = Vec::new();
                for text in &texts {
                    let link = if left_texts.contains(text) {
                        Some(Scalar {
                            owner: left_typ.clone(),
                            index: 0,
                        })
                    } else if right_texts.contains(text) {
                        Some(Scalar {
                            owner: right_typ.clone(),
                            index: 0,
                        })
                    } else {
                        return Ok(None);
                    };
                    links.push((text.clone(), link));
                }
                Ok(Some((obligation, links)))
            }
            ExpKind::Constraint(node) => {
                // Source constraint over rigid scope variables (like
                // `sizeof`); anything else stays `Unsupported`-free `None`.
                let mut expanded_it = (*self.it).clone();
                let expanded = self
                    .program
                    .expanded_constraint(node, &mut expanded_it, self.position);
                let Some(obligation) = crate::conv::constraint_from_ast(&expanded, &expanded_it)
                else {
                    return Ok(None);
                };
                let Some(context) = self.context.clone() else {
                    return Ok(None);
                };
                let view = Type::root(context);
                let mut texts = Vec::new();
                obligation.free_vars(&mut texts);
                let mut links = Vec::with_capacity(texts.len());
                for text in texts {
                    // Local type variables link to their value's witness.
                    let local = self.tyvars.iter().rev().find_map(|(symbol, scalar)| {
                        (self.it.resolve(*symbol) == Some(text.as_str())).then(|| scalar.clone())
                    });
                    if let Some(scalar) = local {
                        links.push((text, Some(scalar)));
                    } else if (&view as &dyn Bindings).rigid(&text).is_some() {
                        links.push((text, None));
                    } else {
                        return Ok(None);
                    }
                }
                Ok(Some((obligation, links)))
            }
            ExpKind::Lit(lit) => match &lit.kind {
                LitKind::True => Ok(Some((MiniCons::True, Vec::new()))),
                LitKind::False => Ok(Some((MiniCons::False, Vec::new()))),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Build the `atom_bool` assumption type for a `guard_mini` result.
    /// Side-linked variables resolve through their views (substitutions
    /// hit before rigid lookup); scope rigids resolve via the function
    /// scope, which needs no substitution entry.
    fn guard_fact(&mut self, exp: &Exp, scope: &Scope, span: Span) -> TypeResult<Option<Type>> {
        let Some((obligation, links)) = self.guard_mini(exp, scope, span)? else {
            return Ok(None);
        };
        let context = self.context.clone();
        if links.iter().any(|(_, link)| link.is_none()) && context.is_none() {
            return Ok(None);
        }
        let mut git = SimpleInterner::new();
        let mut scalars = Vec::new();
        for (text, link) in &links {
            if let Some(scalar) = link {
                scalars.push((git.intern(text), scalar.clone()));
            }
        }
        let constraint = crate::conv::constraint_to_ast(&obligation, &mut git);
        let atom_bool = git.intern("atom_bool");
        let typ = Node::dummy(TypKind::App {
            name: Name::dummy(atom_bool),
            args: vec![Node::dummy(TypArgKind::Bool(Box::new(constraint)))],
        });
        let root = Arc::new(Scheme {
            typschm: TypSchm { quant: vec![], typ },
            interner: Arc::new(git),
        });
        let scope = context.unwrap_or(root.clone());
        Ok(Some(Type {
            root: root.clone(),
            path: Vec::new(),
            scope,
            substitutions: Arc::new(Substitutions {
                types: Vec::new(),
                scalars,
            }),
            implicit_argument: false,
        }))
    }
    /// Assert with continuation scope (OCaml `check_exp` `E_assert` +
    /// `assert_constraint`): the condition checks against `bool`, the
    /// message against `string`; an extracted guard fact
    /// (`atom_bool` conditions, comparisons, `&`/`|`, `E_constraint`,
    /// literals via `guard_fact`) extends the scope for following
    /// statements. No fact: scope unchanged (current behavior). The
    /// result is always `unit`. Dead-code elimination on provably-false
    /// guards is a rewrite, not checking (honest gap).
    fn check_assert(
        &mut self,
        cond: &Exp,
        msg: &Exp,
        scope: &Arc<Scope>,
    ) -> TypeResult<Arc<Scope>> {
        self.weak_facts.set(true);
        let fact = self.condition(cond, scope);
        self.weak_facts.set(false);
        let fact = fact?;
        let message = self.infer(msg, scope)?;
        let string = self.builtin("string");
        self.compatible(&string, &message, msg.span)?;
        Ok(scope.branch(fact.as_ref(), false))
    }
    /// Open existential heads (OCaml existential elimination): each
    /// opening rebinds the binders in a fresh scope, so identical texts
    /// never alias across openings (fresh scope `Arc` = fresh rigid
    /// identity). Dropped constraints are kept as scope-quant markers
    /// for `instantiate` to harvest as assumptions (linear conjuncts
    /// only; `2 ^`-style nonlinear parts stay out — honest gap).
    /// Binder-only inner types (every builtin shape) link fully; outer
    /// variables inside keep original views through preserved
    /// substitutions, truly unbound ones fail honestly downstream.
    /// Fuel-capped; anything left over returns unchanged (never guessed).
    fn open_existential(&self, typ: &Type) -> Type {
        let mut opened = typ.clone();
        let mut markers = Vec::new();
        for _ in 0..8 {
            let TypKind::Exist {
                binders,
                constraint,
                typ: inner,
            } = &opened.node().kind
            else {
                break;
            };
            markers.push((**constraint).clone());
            let shadowed = |symbol: &Symbol| binders.iter().any(|binder| binder.kid == *symbol);
            let substitutions = Arc::new(Substitutions {
                types: opened
                    .substitutions
                    .types
                    .iter()
                    .filter(|(symbol, _)| !shadowed(symbol))
                    .cloned()
                    .collect(),
                scalars: opened
                    .substitutions
                    .scalars
                    .iter()
                    .filter(|(symbol, _)| !shadowed(symbol))
                    .cloned()
                    .collect(),
            });
            let scope = Arc::new(Scheme {
                typschm: TypSchm {
                    quant: binders
                        .iter()
                        .map(|binder| QuantItem::Id(binder.clone()))
                        .collect(),
                    typ: Node::dummy(TypKind::Unknown),
                },
                interner: opened.root.interner.clone(),
            });
            opened = Type {
                root: Arc::new(Scheme {
                    typschm: TypSchm {
                        quant: Vec::new(),
                        typ: (**inner).clone(),
                    },
                    interner: opened.root.interner.clone(),
                }),
                path: Vec::new(),
                scope,
                substitutions,
                implicit_argument: opened.implicit_argument,
            };
        }
        if !markers.is_empty() {
            let scope = opened.scope.clone();
            let mut quant = scope.typschm.quant.clone();
            quant.extend(markers.into_iter().map(QuantItem::Constraint));
            opened = Type {
                root: opened.root.clone(),
                path: opened.path.clone(),
                scope: Arc::new(Scheme {
                    typschm: TypSchm {
                        quant,
                        typ: Node::dummy(TypKind::Unknown),
                    },
                    interner: scope.interner.clone(),
                }),
                substitutions: opened.substitutions.clone(),
                implicit_argument: opened.implicit_argument,
            };
        }
        opened
    }
    /// Check a value against an existential type (subsumption): bind the
    /// binders from the found type and prove the constraint, reusing
    /// `instantiate_relaxed` with a 0-argument callee. Outer variables
    /// (non-binders) pre-link to function-scope wrapper views, so rigid
    /// assumptions (e.g. `'l` in `0 <= 'm < 2 ^ 'l`) resolve; truly
    /// unbound ones fail honestly downstream. Shadowing inside nested
    /// existentials is respected by symbol.
    fn check_existential(&self, expected: &Type, found: &Type, span: Span) -> TypeResult<()> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum OuterKind {
            Int,
            Bool,
        }
        /// Iterative outer-variable collector with shadowing
        /// discipline (no recursion): an explicit stack walks types,
        /// nexps and constraints; nested `Exist` binders extend the
        /// binder stack eagerly with a truncation marker behind their
        /// children (LIFO restores the outer scope exactly).
        enum CollectTask<'a> {
            Typ(&'a Typ),
            NExp(&'a NExp),
            Constraint(&'a Constraint),
            PopScope(usize),
        }
        fn collect_push(
            symbol: Symbol,
            kind: OuterKind,
            binders: &[Symbol],
            out: &mut Vec<(Symbol, OuterKind)>,
        ) {
            if !binders.contains(&symbol) && !out.iter().any(|(bound, _)| bound == &symbol) {
                out.push((symbol, kind));
            }
        }
        fn collect_all(
            typ: &Typ,
            constraint: &Constraint,
            binders: &mut Vec<Symbol>,
            out: &mut Vec<(Symbol, OuterKind)>,
        ) {
            let mut stack = vec![CollectTask::Constraint(constraint), CollectTask::Typ(typ)];
            while let Some(task) = stack.pop() {
                match task {
                    CollectTask::Typ(node) => match &node.kind {
                        TypKind::Var(_) | TypKind::Id(_) | TypKind::Unknown => {}
                        TypKind::App { args, .. } => {
                            for arg in args {
                                match &arg.kind {
                                    TypArgKind::NExp(nexp) => {
                                        stack.push(CollectTask::NExp(nexp));
                                    }
                                    TypArgKind::Typ(inner) => {
                                        stack.push(CollectTask::Typ(inner));
                                    }
                                    TypArgKind::Bool(inner) => {
                                        stack.push(CollectTask::Constraint(inner));
                                    }
                                }
                            }
                        }
                        TypKind::Tuple(items) => {
                            for item in items {
                                stack.push(CollectTask::Typ(item));
                            }
                        }
                        TypKind::Fn { args, ret } => {
                            stack.push(CollectTask::Typ(ret));
                            for arg in args {
                                stack.push(CollectTask::Typ(arg));
                            }
                        }
                        TypKind::Bidir { lhs, rhs } => {
                            stack.push(CollectTask::Typ(lhs));
                            stack.push(CollectTask::Typ(rhs));
                        }
                        TypKind::Exist {
                            binders: inner,
                            constraint,
                            typ,
                        } => {
                            let base = binders.len();
                            binders.extend(inner.iter().map(|binder| binder.kid));
                            stack.push(CollectTask::PopScope(base));
                            stack.push(CollectTask::Typ(typ));
                            stack.push(CollectTask::Constraint(constraint));
                        }
                    },
                    CollectTask::NExp(node) => match &node.kind {
                        NExpKind::Var(symbol) => {
                            collect_push(*symbol, OuterKind::Int, binders, out);
                        }
                        NExpKind::Constant(_) | NExpKind::Id(_) => {}
                        NExpKind::Sum(first, second)
                        | NExpKind::Minus(first, second)
                        | NExpKind::Times(first, second) => {
                            stack.push(CollectTask::NExp(first));
                            stack.push(CollectTask::NExp(second));
                        }
                        NExpKind::Neg(inner) | NExpKind::Exp(inner) => {
                            stack.push(CollectTask::NExp(inner));
                        }
                        NExpKind::If {
                            cond,
                            then_branch,
                            else_branch,
                        } => {
                            stack.push(CollectTask::NExp(then_branch));
                            stack.push(CollectTask::NExp(else_branch));
                            stack.push(CollectTask::Constraint(cond));
                        }
                        NExpKind::App { args, .. } => {
                            for arg in args {
                                stack.push(CollectTask::NExp(arg));
                            }
                        }
                    },
                    CollectTask::Constraint(node) => match &node.kind {
                        ConstraintKind::True | ConstraintKind::False | ConstraintKind::Id(_) => {}
                        ConstraintKind::Var(symbol) => {
                            collect_push(*symbol, OuterKind::Bool, binders, out);
                        }
                        ConstraintKind::Equal(first, second)
                        | ConstraintKind::NotEqual(first, second) => {
                            for arg in [first, second] {
                                match &arg.kind {
                                    TypArgKind::NExp(nexp) => {
                                        stack.push(CollectTask::NExp(nexp));
                                    }
                                    TypArgKind::Typ(inner) => {
                                        stack.push(CollectTask::Typ(inner));
                                    }
                                    TypArgKind::Bool(inner) => {
                                        stack.push(CollectTask::Constraint(inner));
                                    }
                                }
                            }
                        }
                        ConstraintKind::Ge(first, second)
                        | ConstraintKind::Gt(first, second)
                        | ConstraintKind::Le(first, second)
                        | ConstraintKind::Lt(first, second) => {
                            stack.push(CollectTask::NExp(first));
                            stack.push(CollectTask::NExp(second));
                        }
                        ConstraintKind::Set { nexp, .. } => {
                            stack.push(CollectTask::NExp(nexp));
                        }
                        ConstraintKind::And(first, second) | ConstraintKind::Or(first, second) => {
                            stack.push(CollectTask::Constraint(first));
                            stack.push(CollectTask::Constraint(second));
                        }
                        ConstraintKind::Not(inner) => {
                            stack.push(CollectTask::Constraint(inner));
                        }
                        ConstraintKind::App { args, .. } => {
                            for arg in args {
                                match &arg.kind {
                                    TypArgKind::NExp(nexp) => {
                                        stack.push(CollectTask::NExp(nexp));
                                    }
                                    TypArgKind::Typ(inner) => {
                                        stack.push(CollectTask::Typ(inner));
                                    }
                                    TypArgKind::Bool(inner) => {
                                        stack.push(CollectTask::Constraint(inner));
                                    }
                                }
                            }
                        }
                    },
                    CollectTask::PopScope(base) => {
                        binders.truncate(base);
                    }
                }
            }
        }
        let TypKind::Exist {
            binders,
            constraint,
            typ: inner,
        } = &expected.node().kind
        else {
            unreachable!("existential check shape");
        };
        let table = expected.root.interner.clone();
        let mut stack: Vec<Symbol> = binders.iter().map(|binder| binder.kid).collect();
        let mut outers = Vec::new();
        collect_all(inner, constraint, &mut stack, &mut outers);
        // Wrapper views route outer variables to the function scope;
        // wrapper heads never emit (only the argument is followed).
        let head = match &inner.kind {
            TypKind::App { name, .. } => name.clone(),
            _ => Name::dummy(Symbol::DUMMY),
        };
        let mut scalars = Vec::new();
        if let Some(context) = self.context.clone() {
            for (symbol, kind) in outers {
                // Variables already bound by the existential's own view (a
                // callee result with its call substitutions) keep that
                // binding; only the remaining names are function rigids.
                if let Some(bound) = expected.substitutions.get_scalar(symbol) {
                    scalars.push((symbol, bound.clone()));
                    continue;
                }
                let var = match kind {
                    OuterKind::Int => TypArgKind::NExp(Node::dummy(NExpKind::Var(symbol))),
                    OuterKind::Bool => {
                        TypArgKind::Bool(Box::new(Node::dummy(ConstraintKind::Var(symbol))))
                    }
                };
                let owner = Type {
                    root: Arc::new(Scheme {
                        typschm: TypSchm {
                            quant: Vec::new(),
                            typ: Node::dummy(TypKind::App {
                                name: head.clone(),
                                args: vec![Node::dummy(var)],
                            }),
                        },
                        interner: table.clone(),
                    }),
                    path: Vec::new(),
                    scope: context.clone(),
                    substitutions: Arc::new(Substitutions::default()),
                    implicit_argument: false,
                };
                scalars.push((symbol, Scalar { owner, index: 0 }));
            }
        }
        // Found side opens (weakening, sound): evidence carries no
        // obligations. Formal side stays untouched (identity checks).
        let found = self.open_existential(found);
        let fake = Type::root(Arc::new(Scheme {
            typschm: TypSchm {
                quant: binders
                    .iter()
                    .map(|binder| QuantItem::Id(binder.clone()))
                    .chain(std::iter::once(QuantItem::Constraint(
                        (**constraint).clone(),
                    )))
                    .collect(),
                typ: Node::dummy(TypKind::Fn {
                    args: Vec::new(),
                    ret: Box::new((**inner).clone()),
                }),
            },
            interner: table,
        }));
        let mut fake = fake;
        fake.substitutions = Arc::new(Substitutions {
            types: Vec::new(),
            scalars,
        });
        self.instantiate_relaxed(fake, &[], Some(&found), span)?;
        Ok(())
    }
    /// Harvest existential-marker constraints from operand scopes
    /// (planted by `open_existential`): dropped callee bounds re-enter
    /// proofs as assumptions with the opening's identities. Marker
    /// scopes carry a dummy `Unknown` type (never a real scheme), so
    /// ordinary scheme constraints are never double-added. Powers retain
    /// congruence through the SMT emitter's conservative UF abstraction;
    /// unsupported applications remain outside these assumptions.
    fn marker_assumptions<'a>(&self, typs: &[&'a Type]) -> Vec<View<'a>> {
        /// Iterative constraint shape walk (no recursion): reports
        /// whether the subtree holds anything the SMT emitter rejects
        /// (applications and unresolved names) and collects free
        /// variables. One explicit stack over nexps, arguments and
        /// constraints; early exit on the first symbolic node.
        fn constraint_shape(node: &Constraint) -> (bool, Vec<Symbol>) {
            enum Task<'a> {
                Nexp(&'a NExp),
                Arg(&'a TypArg),
                Constraint(&'a Constraint),
            }
            let mut symbolic = false;
            let mut free = Vec::new();
            let mut stack = vec![Task::Constraint(node)];
            while let Some(task) = stack.pop() {
                if symbolic {
                    break;
                }
                match task {
                    Task::Nexp(current) => match &current.kind {
                        NExpKind::Constant(_) => {}
                        NExpKind::Var(symbol) => {
                            if !free.contains(symbol) {
                                free.push(*symbol);
                            }
                        }
                        // Names need proof variables the canonical
                        // context does not carry: fail the whole query
                        // instead of just skipping, hence symbolic.
                        NExpKind::Id(_) => symbolic = true,
                        NExpKind::Sum(first, second)
                        | NExpKind::Minus(first, second)
                        | NExpKind::Times(first, second) => {
                            stack.push(Task::Nexp(first));
                            stack.push(Task::Nexp(second));
                        }
                        NExpKind::Neg(inner) => stack.push(Task::Nexp(inner)),
                        NExpKind::Exp(exponent) => stack.push(Task::Nexp(exponent)),
                        NExpKind::If {
                            cond,
                            then_branch,
                            else_branch,
                        } => {
                            stack.push(Task::Nexp(then_branch));
                            stack.push(Task::Nexp(else_branch));
                            stack.push(Task::Constraint(cond));
                        }
                        NExpKind::App { .. } => symbolic = true,
                    },
                    Task::Arg(current) => match &current.kind {
                        TypArgKind::NExp(nexp) => stack.push(Task::Nexp(nexp)),
                        TypArgKind::Typ(_) => symbolic = true,
                        TypArgKind::Bool(inner) => stack.push(Task::Constraint(inner)),
                    },
                    Task::Constraint(current) => match &current.kind {
                        ConstraintKind::True | ConstraintKind::False => {}
                        ConstraintKind::Var(symbol) => {
                            if !free.contains(symbol) {
                                free.push(*symbol);
                            }
                        }
                        ConstraintKind::Id(_) => symbolic = true,
                        ConstraintKind::Set { nexp, .. } => stack.push(Task::Nexp(nexp)),
                        ConstraintKind::Equal(first, second)
                        | ConstraintKind::NotEqual(first, second) => {
                            stack.push(Task::Arg(first));
                            stack.push(Task::Arg(second));
                        }
                        ConstraintKind::Ge(first, second)
                        | ConstraintKind::Gt(first, second)
                        | ConstraintKind::Le(first, second)
                        | ConstraintKind::Lt(first, second) => {
                            stack.push(Task::Nexp(first));
                            stack.push(Task::Nexp(second));
                        }
                        ConstraintKind::And(first, second) | ConstraintKind::Or(first, second) => {
                            stack.push(Task::Constraint(first));
                            stack.push(Task::Constraint(second));
                        }
                        ConstraintKind::Not(inner) => stack.push(Task::Constraint(inner)),
                        ConstraintKind::App { .. } => symbolic = true,
                    },
                }
            }
            (symbolic, free)
        }
        let mut assumptions = Vec::new();
        let mut pending: Vec<&'a Type> = typs.to_vec();
        let mut seen = HashSet::new();
        while let Some(typ) = pending.pop() {
            let key = (Arc::as_ptr(&typ.scope), Arc::as_ptr(&typ.substitutions));
            if !seen.insert(key) {
                continue;
            }
            // Bounds can depend on values returned by earlier calls. Follow
            // their existing substitution views to retain that evidence with
            // its original binder identities, without copying AST subtrees.
            pending.extend(typ.substitutions.types.iter().map(|(_, owner)| owner));
            pending.extend(
                typ.substitutions
                    .scalars
                    .iter()
                    .map(|(_, scalar)| &scalar.owner),
            );
            if !matches!(&typ.scope.typschm.typ.kind, TypKind::Unknown) {
                continue;
            }
            // Marker syntax belongs to its opening's symbol table. Alias
            // views use a different table and telescope substitutions; the
            // original marker owner is reached through those substitutions.
            // Emitting this view's marker under the alias table can resolve
            // unrelated symbols or fail on a symbol absent from that table.
            if !Arc::ptr_eq(&typ.root.interner, &typ.scope.interner) {
                continue;
            }
            let binders: Vec<Symbol> = typ
                .scope
                .typschm
                .quant
                .iter()
                .filter_map(|item| match item {
                    QuantItem::Id(binder) => Some(binder.kid),
                    QuantItem::Constraint(_) => None,
                })
                .collect();
            for item in &typ.scope.typschm.quant {
                let QuantItem::Constraint(marker) = item else {
                    continue;
                };
                // Iterative `And` flattening (no recursion); `Or` stays
                // whole like before.
                let mut conjuncts = vec![marker];
                let mut flat = Vec::new();
                while let Some(current) = conjuncts.pop() {
                    match &current.kind {
                        ConstraintKind::And(first, second) => {
                            conjuncts.push(first);
                            conjuncts.push(second);
                        }
                        _ => flat.push(current),
                    }
                }
                for conjunct in flat {
                    let (symbolic, free) = constraint_shape(conjunct);
                    if symbolic {
                        continue;
                    }
                    // Outer variables need explicit substitution views;
                    // unlinked names stay outside this marker's assumptions.
                    if free.iter().any(|symbol| {
                        !binders.contains(symbol) && typ.substitutions.get_scalar(*symbol).is_none()
                    }) {
                        continue;
                    }
                    assumptions.push(View {
                        constraint: conjunct,
                        context: typ.proof_context(),
                        negated: false,
                    });
                }
            }
        }
        assumptions
    }
    /// Vector-LHS target type (Lane 41 hook for `assignment_type`).
    ///
    /// Pipeline, load-bearing order:
    /// 1. Wurzel rekursiv bestimmen (`assignment_type` auf der Basis, also
    ///    auch geschachtelt) und Synonyme expandieren (`resolve`);
    /// 2. Indexausdrücke voll inferieren (`infer`, int-artig via
    ///    `lexp_bounds::check_index_typ`) und als `Nexp` synthetisieren
    ///    (`vector_lexp_bridge`, Lücken als `Unsupported`);
    /// 3. Bounds **zuerst** beweisen (`prove_vector_bounds`);
    /// 4. erst danach prüft der `Assign`-Arm den RHS gegen den hier
    ///    bestimmten Element-/Slice-Typ.
    ///
    /// `check_assign`-Priorität: Jeder LExp-Fehler kehrt per `?` zurück,
    /// bevor der RHS-Task überhaupt eingereiht wird — ein RHS-Fehler kann
    /// einen Index-Fehler nie umgehen.
    ///
    /// Offene Lücke ohne Raten: `VectorRange` braucht die Default-Order
    /// (`Env.get_default_order`, kein impliziter Fallback); `Program` kennt
    /// keine, darum meldet der Slice-Pfad nach geprüfter Wurzel und
    /// geprüften Grenzen laut `Unsupported`.
    fn vector_target_type(&mut self, lexp: &LExp, scope: &Scope) -> TypeResult<Type> {
        use crate::lexp_bounds::{check_index_typ, check_slice_root_len, VectorRoot};
        use crate::vector_lexp::{as_vector, as_vector_range};
        use crate::vector_lexp_bridge as bridge;
        if let Some((base, index)) = as_vector(lexp) {
            let base_typ = self.assignment_type(base, scope)?;
            let root = self.resolve(&base_typ)?;
            // `r[FIELD] = e` on a bitfield assigns the field's slice.
            if let ExpKind::Id(field) = &index.kind {
                let field_text = self.name(field)?.to_owned();
                if let Some(getter) = self.bitfield_accessor("_get", &root, &field_text)? {
                    return self.resolve(&getter.child(Step::Result));
                }
            }
            let mut union = crate::vector_lexp::Union::over(vec![
                &*root.root.interner as &dyn sail_ast::Interner,
                &*root.scope.interner as &dyn sail_ast::Interner,
                &*self.it as &dyn sail_ast::Interner,
            ]);
            let vector_root = bridge::vector_root_of_canonical(root.node(), &mut union, lexp.span)?;
            // Len folgt der Substitutions-Kette (Helper; NLL beendet die
            // obige Union-Ausleihe vor dem &mut-Borrow).
            let len_nexp = self.vector_len_nexp(&root, lexp.span)?;
            let inferred = self.infer(index, scope)?;
            let index_typ = self.resolve(&inferred)?;
            let mini_index_typ = bridge::mini_typ_of_canonical(
                index_typ.node(),
                &*index_typ.root.interner,
                index.span,
            )?;
            check_index_typ(&mini_index_typ, index.span)?;
            // Bounds als Mini-Obligation; Variablen-Linking über
            // `prove_vector_bounds` (Scope-Locals, sonst Funktionssicht).
            // Frische Union: die obige Ausleihe ist durch NLL beendet.
            // Index expression symbols belong to the checked definition's
            // interner alone; other tables would reuse the same numbers for
            // different names.
            let mut union2 =
                crate::vector_lexp::Union::over(vec![&*self.it as &dyn sail_ast::Interner]);
            // Syntactic indices map directly; any other expression is bound
            // through the numeric witness of its inferred type.
            let witness_scope;
            let context_view = self.context.as_ref().map(|s| Type::root(s.clone()));
            let mapped = bridge::index_nexp_from_exp(index, &mut union2).and_then(|nexp| {
                let mut names = Vec::new();
                nexp.free_vars(&mut names);
                let linkable = names.iter().all(|name| {
                    scope.get(name).is_some()
                        || context_view
                            .as_ref()
                            .is_some_and(|view| (view as &dyn Bindings).rigid(name).is_some())
                });
                if linkable {
                    Ok(nexp)
                } else {
                    Err(unsupported(index.span, "vector index names a global value"))
                }
            });
            let (index_nexp, bound_scope) = match mapped {
                Ok(nexp) => (nexp, scope),
                Err(error) if matches!(error.kind, TypeErrorKind::Unsupported(_)) => {
                    witness_scope = Scope {
                        locals: vec![("__vector_index".to_owned(), (index_typ.clone(), false))],
                        facts: Vec::new(),
                        parent: Some(Arc::new(scope.clone())),
                    };
                    (crate::nexp::Nexp::Var("__vector_index".to_owned()), &witness_scope)
                }
                Err(error) => return Err(error),
            };
            self.prove_vector_bounds(
                &crate::lexp_bounds::index_obligation(&index_nexp, &len_nexp),
                bound_scope,
                lexp.span,
                "vector index bounds",
            )?;
            // Das Element aus der substituierten Sicht lesen (call-result
            // Bindings anwenden, z.B. `'a := bits(8)` aus `vector_init`):
            // die rohe AST-Ansicht trägt noch die unsubstituierte Variable.
            // WICHTIG: kein Mini-Roundtrip (`ast_of_mini_typ` baut frische
            // Tabellen ohne Scope und zerreisst rigide Identitäten); die
            // substituierte Sicht direkt zurückgeben.
            match &vector_root {
                VectorRoot::Bitvector { .. } => return Ok(self.builtin("bit")),
                VectorRoot::Vector { .. } => {
                    let elem_view = root.child(Step::TypeArgument(1)).resolved();
                    return Ok(elem_view);
                }
            }
        }
        if let Some((base, first, second)) = as_vector_range(lexp) {
            let base_typ = self.assignment_type(base, scope)?;
            let root = self.resolve(&base_typ)?;

            let mut union = crate::vector_lexp::Union::over(vec![
                &*root.root.interner as &dyn sail_ast::Interner,
                &*root.scope.interner as &dyn sail_ast::Interner,
                &*self.it as &dyn sail_ast::Interner,
            ]);
            let vector_root = bridge::vector_root_of_canonical(root.node(), &mut union, lexp.span)?;
            // Ein `vector`-Root ist hier ein Mismatch (OCaml:
            // "Cannot assign slice of non vector type").
            let _len = check_slice_root_len(&vector_root, lexp.span)?;
            // Default-Order (OCaml `Env.get_default_order`); ohne gesetzte
            // Order ehrlich `Unsupported` (kein impliziter Fallback).
            let order = self.program.default_order(self.position).ok_or_else(|| {
                unsupported(
                    lexp.span,
                    "vector slice assignment needs a default order (Env.get_default_order); no implicit fallback",
                )
            })?;
            let order = match order {
                sail_ast::Order::Inc => crate::nexp::MiniOrder::Inc,
                sail_ast::Order::Dec => crate::nexp::MiniOrder::Dec,
            };
            // Len folgt der Substitutions-Kette (Helper; NLL beendet die
            // obige Union-Ausleihe vor dem &mut-Borrow).
            let len_nexp = self.vector_len_nexp(&root, lexp.span)?;
            let mut first_nexp = None;
            let mut second_nexp = None;
            for (bound, slot) in [(first, &mut first_nexp), (second, &mut second_nexp)] {
                let inferred = self.infer(bound, scope)?;
                let bound_typ = self.resolve(&inferred)?;
                let mini = bridge::mini_typ_of_canonical(
                    bound_typ.node(),
                    &*bound_typ.root.interner,
                    bound.span,
                )?;
                check_index_typ(&mini, bound.span)?;
                let mut bunion =
                    crate::vector_lexp::Union::over(vec![&*self.it as &dyn sail_ast::Interner]);
                *slot = Some(bridge::index_nexp_from_exp(bound, &mut bunion)?);
            }
            let (first_nexp, second_nexp) = (first_nexp.expect("set"), second_nexp.expect("set"));
            let (slice_len, check) =
                crate::lexp_bounds::slice_obligation(&first_nexp, &second_nexp, &len_nexp, order);
            self.prove_vector_bounds(&check, scope, lexp.span, "vector slice bounds")?;
            // Slice-Typ mit verlinkten Substitutionen (keine Orphan-Sicht):
            // `bits(slice_len)` frisch, jede Variable auf Local-Witness
            // abgebildet (unabgebildete = ehrlich `Unsupported`).
            let mut git = SimpleInterner::new();
            let mut texts = Vec::new();
            slice_len.free_vars(&mut texts);
            texts.sort();
            texts.dedup();
            let mut scalars = Vec::new();
            for text in &texts {
                let (typ, _) = scope.get(text).ok_or_else(|| {
                    unsupported(
                        lexp.span,
                        format!("slice length variable {text} is unbound"),
                    )
                })?;
                match &typ.node().kind {
                    TypKind::App { args, .. } if args.len() == 1 => {
                        let TypArgKind::NExp(_) = &args[0].kind else {
                            return Err(unsupported(
                                lexp.span,
                                format!("slice length variable {text} has no numeric witness"),
                            ));
                        };
                        let sym = git.intern(text);
                        scalars.push((
                            sym,
                            Scalar {
                                owner: typ.clone(),
                                index: 0,
                            },
                        ));
                    }
                    _ => {
                        return Err(unsupported(
                            lexp.span,
                            format!("slice length variable {text} has no numeric witness"),
                        ));
                    }
                }
            }
            let len_node = crate::conv::nexp_to_ast(&slice_len, &mut git);
            let bits = git.intern("bits");
            let typ = Node::dummy(TypKind::App {
                name: Name::dummy(bits),
                args: vec![Node::dummy(TypArgKind::NExp(len_node))],
            });
            let root = Arc::new(Scheme {
                typschm: TypSchm { quant: vec![], typ },
                interner: Arc::new(git),
            });
            return Ok(Type {
                root: root.clone(),
                path: Vec::new(),
                scope: root,
                substitutions: Arc::new(Substitutions {
                    types: Vec::new(),
                    scalars,
                }),
                implicit_argument: false,
            });
        }
        unreachable!("vector_target_type called on a non-vector LExp");
    }
    /// Check write permission independently of RHS inference. Reading a local
    /// or a register does not grant permission to mutate an immutable binding.
    /// Mirrors the `has_typ`/permission split of `bind_assignment`.
    ///
    /// Vector hook (Lane 41, `LExpKind::Vector` / `VectorRange`): the
    /// catch-all below routes vector targets through [`Self::vector_target_type`]
    /// (root expand, index infer, bounds first, RHS last); every other
    /// non-`Id`/`Tuple` target stays `Unsupported`.
    fn assignment_type(&mut self, lexp: &LExp, scope: &Scope) -> TypeResult<Type> {
        enum Target<'a> {
            Enter(&'a LExp),
            Tuple(usize),
        }
        let mut work = vec![Target::Enter(lexp)];
        let mut results = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Target::Tuple(count) => {
                    let items = results.split_off(results.len() - count);
                    results.push(self.tuple_type(items));
                }
                Target::Enter(lexp) => {
                    match &lexp.kind {
                        LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
                            let text = self.name(name)?;
                            let declared = match scope.get(text) {
                            Some((typ, true)) => typ.clone(),
                            Some((_, false)) => return Err(unsupported(lexp.span, "cannot assign to an immutable local")),
                            None => self.program.register(text, self.position).cloned().map(Type::root)
                                .ok_or_else(|| unsupported(lexp.span, "assignment requires an existing mutable local or register"))?,
                        };
                            let declared = if let LExpKind::Typ { typ, .. } = &lexp.kind {
                                let annotated = self.annotation(typ)?;
                                self.compatible(&declared, &annotated, lexp.span)?;
                                annotated
                            } else {
                                declared
                            };
                            if self.recorder.is_some() {
                                self.record_lexp(lexp, &declared);
                            }
                            results.push(declared);
                        }
                        LExpKind::Field { lexp: base, field } => {
                            // OCaml `infer_lexp` LE_field: the base must be a
                            // writable record; the target has the field type.
                            let base_typ = self.assignment_type(base, scope)?;
                            let base_typ = self.resolve(&base_typ)?;
                            let record_name = base_typ
                                .name()
                                .map(str::to_owned)
                                .filter(|name| self.program.record(name, self.position).is_some());
                            let Some(record_name) = record_name else {
                                return Err(TypeError {
                                    span: lexp.span,
                                    kind: TypeErrorKind::QuantError(format!(
                                        "The type {} is not a record",
                                        base_typ.label()
                                    )),
                                });
                            };
                            let field_name = self.name(field)?.to_owned();
                            let typ = self.struct_field_type(
                                &record_name,
                                &field_name,
                                &base_typ,
                                field.span,
                            )?;
                            results.push(self.resolve(&typ)?);
                        }
                        LExpKind::VectorConcat(parts) => {
                            // OCaml `infer_lexp` LE_vector_concat: every part is
                            // a bitvector target; the target is as wide as all
                            // parts together. Lengths link as whole scalars.
                            let mut it = SimpleInterner::new();
                            let mut scalars = Vec::new();
                            let mut sum: Option<NExp> = None;
                            for (index, part) in parts.iter().enumerate() {
                                let typ = self.assignment_type(part, scope)?;
                                let typ = self.resolve(&typ)?;
                                let TypKind::App { name, args } = &typ.node().kind else {
                                    return Err(unsupported(part.span, "vector concat target is not a bitvector"));
                                };
                                let is_bitvector = typ
                                    .root
                                    .interner
                                    .resolve(name.symbol)
                                    .is_some_and(|n| n == "bitvector" || n == "bits");
                                if !is_bitvector
                                    || !matches!(args.first().map(|a| &a.kind), Some(TypArgKind::NExp(_)))
                                {
                                    return Err(unsupported(part.span, "vector concat target is not a bitvector"));
                                }
                                let symbol = it.intern(&format!("__part{index}"));
                                scalars.push((symbol, Scalar { owner: typ, index: 0 }.resolved()));
                                let var = Node::dummy(NExpKind::Var(symbol));
                                sum = Some(match sum {
                                    None => var,
                                    Some(total) => {
                                        Node::dummy(NExpKind::Sum(Box::new(total), Box::new(var)))
                                    }
                                });
                            }
                            let head = Name::dummy(it.intern("bits"));
                            let typ = Node::dummy(TypKind::App {
                                name: head,
                                args: vec![Node::dummy(TypArgKind::NExp(
                                    sum.ok_or_else(|| unsupported(lexp.span, "empty vector concat target"))?,
                                ))],
                            });
                            let mut joined = Type::root(Arc::new(Scheme {
                                typschm: TypSchm { quant: Vec::new(), typ },
                                interner: Arc::new(it),
                            }));
                            joined.substitutions = Arc::new(Substitutions {
                                types: Vec::new(),
                                scalars,
                            });
                            results.push(joined);
                        }
                        LExpKind::Deref(inner) => {
                            let reference = self.infer(inner, scope)?;
                            let reference = self.resolve(&reference)?;
                            if reference.name() != Some("register") {
                                return Err(TypeError::mismatch(
                                    lexp.span,
                                    "register".to_owned(),
                                    reference.label(),
                                ));
                            }
                            results.push(self.resolve(&reference.child(Step::TypeArgument(0)))?);
                        }
                        LExpKind::Tuple(items) => {
                            work.push(Target::Tuple(items.len()));
                            work.extend(items.iter().rev().map(Target::Enter));
                        }
                        LExpKind::Vector { .. } | LExpKind::VectorRange { .. } => {
                            results.push(self.vector_target_type(lexp, scope)?);
                        }
                        _ => {
                            return Err(unsupported(
                                lexp.span,
                                "field or reference assignment target",
                            ))
                        }
                    }
                }
            }
        }
        Ok(results.pop().expect("assignment target type"))
    }
    /// Foreach body scope: immutable index with borrowed `from <= index <= to`
    /// (ascending) bounds. Bounds stay borrowed singleton views; the index gets
    /// its own rigid witness. Only `int` bounds are elaborated here.
    fn iteration_scope(
        &mut self,
        var: &Name,
        lower: Type,
        upper: Type,
        parent: &Arc<Scope>,
    ) -> TypeResult<Arc<Scope>> {
        let lower = self.resolve(&lower)?;
        let upper = self.resolve(&upper)?;
        if lower.name() != Some("atom") || upper.name() != Some("atom") {
            return Err(unsupported(
                var.span,
                "foreach bounds require numeric singleton elaboration",
            ));
        }
        let integer = self.builtin("int");
        let index = self.open_value(&integer);
        // OCaml `E_for` binds the loop variable's witness as `'loop_<var>`.
        let loop_name = format!("'loop_{}", self.name(var)?);
        if let Some(symbol) = self.it.lookup(&loop_name) {
            self.tyvars.retain(|(bound, _)| *bound != symbol);
            self.tyvars.push((
                symbol,
                Scalar {
                    owner: index.clone(),
                    index: 0,
                },
            ));
        }
        // Share one condition shape per checker: the bound/index symbols and
        // the constraint AST are identical for every loop, only the owners
        // bound in substitutions differ per level.
        if self.foreach_template.is_none() {
            let mut interner = SimpleInterner::new();
            let lo = interner.intern("__lower");
            let mid = interner.intern("__index");
            let hi = interner.intern("__upper");
            let name = Name::dummy(interner.intern("atom_bool"));
            let predicate = Node::dummy(ConstraintKind::And(
                Box::new(Node::dummy(ConstraintKind::Le(
                    Node::dummy(NExpKind::Var(lo)),
                    Node::dummy(NExpKind::Var(mid)),
                ))),
                Box::new(Node::dummy(ConstraintKind::Le(
                    Node::dummy(NExpKind::Var(mid)),
                    Node::dummy(NExpKind::Var(hi)),
                ))),
            ));
            let condition = Type::owned(
                Node::dummy(TypKind::App {
                    name,
                    args: vec![Node::dummy(TypArgKind::Bool(Box::new(predicate)))],
                }),
                Arc::new(interner),
            );
            self.foreach_template = Some(ForeachTemplate {
                condition,
                lower: lo,
                index: mid,
                upper: hi,
            });
        }
        let template = self
            .foreach_template
            .as_ref()
            .expect("foreach condition template")
            .clone();
        let mut condition = template.condition.clone();
        condition.substitutions = Arc::new(Substitutions {
            types: Vec::new(),
            scalars: vec![
                (
                    template.lower,
                    Scalar {
                        owner: lower,
                        index: 0,
                    },
                ),
                (
                    template.index,
                    Scalar {
                        owner: index.clone(),
                        index: 0,
                    },
                ),
                (
                    template.upper,
                    Scalar {
                        owner: upper,
                        index: 0,
                    },
                ),
            ],
        });
        Ok(Arc::new(Scope {
            locals: vec![(self.name(var)?.to_owned(), (index, false))],
            facts: vec![(
                Scalar {
                    owner: condition,
                    index: 0,
                },
                false,
            )],
            parent: Some(parent.clone()),
        }))
    }
    /// OCaml `check_exp` E_struct/E_struct_update premise: resolve synonyms
    /// of the expected type; the head must be a declared record
    /// (`ExpectedMustBeRecord`), else "not a record".
    fn struct_expected_record(&self, expected: &Type, span: Span) -> TypeResult<String> {
        let resolved = self.resolve(expected)?;
        if let Some(name) = resolved.name() {
            if self.program.record(name, self.position).is_some() {
                return Ok(name.to_owned());
            }
        }
        Err(TypeError::mismatch(
            span,
            "record".to_owned(),
            format!("not a record: {}", resolved.label()),
        ))
    }

    /// Field payload through the record accessor contract (OCaml
    /// `Env.get_accessor`, Lane-08 `Program::accessor`): the preserved
    /// `Fn(record) -> field` scheme unified against the base type.
    fn struct_field_type(
        &self,
        record: &str,
        field: &str,
        base: &Type,
        span: Span,
    ) -> TypeResult<Type> {
        let scheme = self
            .program
            .accessor(record, field, self.position)
            .cloned()
            .ok_or_else(|| TypeError::unbound_var(span, field.to_owned()))?;
        let accessor = Type::root(scheme);
        let instantiated = self.instantiate(accessor, std::slice::from_ref(base), None, span)?;
        Ok(self.open_existential(&instantiated.child(Step::Result)))
    }

    /// Field read `record.field` (OCaml `infer_exp` `E_field`): infer the
    /// base, resolve to a declared record, unify the accessor telescope
    /// against the base via the existing `struct_field_type`. Unknown
    /// records are hard `QuantError`, unknown fields `UnboundVar`;
    /// generic (constrained) accessors prove through `instantiate`.
    fn field_type(
        &mut self,
        record: &Exp,
        field: &Name,
        scope: &Scope,
        span: Span,
    ) -> TypeResult<Type> {
        let inferred = self.infer(record, scope)?;
        let base = self.resolve(&inferred)?;
        let record_name = base
            .name()
            .map(str::to_owned)
            .filter(|name| self.program.record(name, self.position).is_some());
        let Some(record_name) = record_name else {
            return Err(TypeError {
                span,
                kind: TypeErrorKind::QuantError(format!(
                    "The type {} is not a record",
                    base.label()
                )),
            });
        };
        let field_name = self.name(field)?;
        self.struct_field_type(&record_name, field_name, &base, field.span)
    }
    fn compatible(&self, expected: &Type, found: &Type, span: Span) -> TypeResult<()> {
        let mut pairs = vec![(expected.clone(), found.clone())];
        while let Some((expected, found)) = pairs.pop() {
            let expected = self.resolve(&expected)?;
            let found = self.resolve(&found)?;
            // Existential expected: subsumption (bind binders from
            // found, prove the constraint). Existential found: open
            // (fresh scope per opening, constraints weakened).
            if matches!(&expected.node().kind, TypKind::Exist { .. }) {
                self.check_existential(&expected, &found, span)?;
                continue;
            }
            if matches!(&found.node().kind, TypKind::Exist { .. }) {
                let found = self.open_existential(&found);
                pairs.push((expected, found));
                continue;
            }
            if matches!(
                (&expected.node().kind, expected.name(), found.name()),
                (TypKind::Id(_), Some("int"), Some("atom" | "range" | "nat"))
                    | (TypKind::Id(_), Some("bool"), Some("atom_bool"))
            ) {
                continue;
            }
            if let (TypKind::Id(_), Some(base), Some(refined)) =
                (&expected.node().kind, expected.name(), found.name())
            {
                if matches!(base, "unit" | "bool" | "int" | "real" | "string")
                    && matches!(refined, "atom" | "atom_bool" | "bitvector")
                {
                    return Err(mismatch(span, &expected, &found));
                }
            }
            match (&expected.node().kind, &found.node().kind) {
                (TypKind::Id(_), TypKind::Id(_)) => {
                    if expected.name() != found.name() {
                        return Err(mismatch(span, &expected, &found));
                    }
                }
                (TypKind::Var(a), TypKind::Var(b))
                    if Arc::ptr_eq(&expected.scope, &found.scope)
                        && expected.root.interner.resolve(*a)
                            == found.root.interner.resolve(*b) => {}
                (TypKind::Tuple(a), TypKind::Tuple(b)) => {
                    if a.len() != b.len() {
                        return Err(arity(span, a.len(), b.len()));
                    }
                    for i in 0..a.len() {
                        pairs.push((expected.child(Step::Tuple(i)), found.child(Step::Tuple(i))));
                    }
                }
                (TypKind::App { args: aa, .. }, TypKind::App { args: ba, .. })
                    if expected.name() == found.name() =>
                {
                    if aa.len() != ba.len() {
                        return Err(arity(span, aa.len(), ba.len()));
                    }
                    for (i, (a, b)) in aa.iter().zip(ba).enumerate() {
                        match (&a.kind, &b.kind) {
                            (TypArgKind::Typ(_), TypArgKind::Typ(_)) => pairs.push((
                                expected.child(Step::TypeArgument(i)),
                                found.child(Step::TypeArgument(i)),
                            )),
                            _ => {
                                self.scalar_equal(
                                    &Scalar {
                                        owner: expected.clone(),
                                        index: i,
                                    },
                                    &Scalar {
                                        owner: found.clone(),
                                        index: i,
                                    },
                                    span,
                                )?;
                            }
                        }
                    }
                }
                (TypKind::Unknown, _) | (_, TypKind::Unknown) => {
                    return Err(unsupported(span, "unresolved type placeholder"))
                }
                (TypKind::App { .. } | TypKind::Exist { .. }, _)
                | (_, TypKind::App { .. } | TypKind::Exist { .. }) => return Err(unsupported(
                    span,
                    "type synonym expansion, refinement subsumption and dependent argument solving",
                )),
                _ => return Err(mismatch(span, &expected, &found)),
            }
        }
        Ok(())
    }
    fn pattern(
        &mut self,
        pat: &Pat,
        typ: Type,
        mutable: bool,
        scope: &mut Scope,
    ) -> TypeResult<()> {
        let mut work = vec![(pat, typ)];
        while let Some((pat, typ)) = work.pop() {
            let typ = self.resolve(&typ)?;
            if self.recorder.is_some() {
                self.record_pat(pat, &typ);
            }
            match &pat.kind {
                PatKind::Wild => {}
                PatKind::Id(name) => {
                    let name_text = self.name(name)?;
                    if let Some(value) = self.program.enumeration_member(name_text, self.position) {
                        self.compatible(&typ, &Type::root(value.clone()), pat.span)?;
                    } else {
                        if self.program.register(name_text, self.position).is_some() {
                            return Err(TypeError {
                                span: pat.span,
                                kind: TypeErrorKind::RegisterError(format!(
                                    "cannot shadow register '{name_text}' in a pattern"
                                )),
                            });
                        }
                        let typ = self.open_value(&typ);
                        if scope.locals.iter().any(|(bound, _)| bound == name_text) {
                            return Err(unsupported(pat.span, "nonlinear pattern binding"));
                        }
                        scope.locals.push((name_text.to_owned(), (typ, mutable)));
                    }
                }
                PatKind::As { pat, id } => {
                    let typ = self.open_value(&typ);
                    scope
                        .locals
                        .push((self.name(id)?.to_owned(), (typ.clone(), mutable)));
                    work.push((pat, typ));
                }
                PatKind::Typ {
                    typ: annotation,
                    pat,
                } => {
                    let annotated = self.annotation(annotation)?;
                    self.compatible(&typ, &annotated, pat.span)?;
                    work.push((pat, annotated));
                }
                PatKind::Tuple(items) => {
                    let TypKind::Tuple(types) = &typ.node().kind else {
                        return Err(unsupported(
                            pat.span,
                            "tuple pattern against a type synonym",
                        ));
                    };
                    if items.len() != types.len() {
                        return Err(arity(pat.span, types.len(), items.len()));
                    }
                    for (i, pat) in items.iter().enumerate().rev() {
                        work.push((pat, typ.child(Step::Tuple(i))));
                    }
                }
                PatKind::Lit(lit) if self.flow_literal(lit, &typ, scope)? => {}
                PatKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    // OCaml binds `bool`/`int` values at their declared
                    // type; an opened fresh witness must not turn the
                    // literal match into a proof obligation.
                    let scrutinee = match self.opened_value_base(&typ) {
                        Some(name) => self.builtin(name),
                        None => typ,
                    };
                    self.compatible(&scrutinee, &actual, pat.span)?;
                }
                PatKind::Var { pat, typ_pat } => {
                    let typ = self.open_value(&typ);
                    self.bind_type_pattern(typ_pat, &typ)?;
                    work.push((pat, typ));
                }
                PatKind::Vector(items) => {
                    // OCaml `bind_pat` `P_vector`: every element binds
                    // against `bit` (non-bitvectors are the BVS TODO on
                    // both sides); the width must match. Constant widths
                    // decide syntactically, rigid lengths stay honest
                    // `Unsupported` (solver equality is future work).
                    let bit = self.builtin("bit");
                    for sub in items.iter().rev() {
                        work.push((sub, bit.clone()));
                    }
                    let total = items.len() as i128;
                    let have = self
                        .vector_len_nexp(&typ, pat.span)
                        .ok()
                        .as_ref()
                        .and_then(crate::nexp::Nexp::as_const);
                    let Some(have) = have else {
                        return Err(unsupported(pat.span, "non-constant vector pattern width"));
                    };
                    if have != total {
                        return Err(arity(pat.span, have as usize, total as usize));
                    }
                }
                PatKind::VectorConcat(items) => {
                    // OCaml `bind_vector_concat_generic` (checking mode):
                    // the scrutinee length must be constant; every
                    // subpattern contributes a constant width (literals
                    // via their type, vectors via arity); at most one
                    // wildcard takes the remainder; the sum must match.
                    let total = match self
                        .vector_len_nexp(&typ, pat.span)
                        .ok()
                        .as_ref()
                        .and_then(crate::nexp::Nexp::as_const)
                    {
                        Some(total) => total,
                        None => {
                            return Err(unsupported(
                                pat.span,
                                "non-constant vector concat pattern width",
                            ));
                        }
                    };
                    // Fresh constant slice types are self-contained (no
                    // variables, no substitutions needed).
                    let slice = |width: i128| {
                        let mut interner = SimpleInterner::new();
                        let name = Name::dummy(interner.intern("bits"));
                        let len = Node::dummy(NExpKind::Constant(width.to_string()));
                        Type::owned(
                            Node::dummy(TypKind::App {
                                name,
                                args: vec![Node::dummy(TypArgKind::NExp(len))],
                            }),
                            Arc::new(interner),
                        )
                    };
                    let bit = self.builtin("bit");
                    let mut known: i128 = 0;
                    let mut wildcards = 0u32;
                    let mut pinned = Vec::with_capacity(items.len());
                    // Binding variables of unclear width (`x @ 0b1`).
                    let mut unclear: Vec<&Pat> = Vec::new();
                    for sub in items {
                        match &sub.kind {
                            PatKind::Wild => {
                                wildcards += 1;
                                pinned.push(None);
                            }
                            PatKind::Id(_) | PatKind::As { .. } => {
                                wildcards += 1;
                                unclear.push(sub);
                                pinned.push(None);
                            }
                            PatKind::Typ { typ: annotation, .. } => {
                                let annotated = self.annotation(annotation)?;
                                let annotated = self.resolve(&annotated)?;
                                let width = self
                                    .vector_len_nexp(&annotated, sub.span)
                                    .ok()
                                    .as_ref()
                                    .and_then(crate::nexp::Nexp::as_const);
                                let Some(width) = width else {
                                    return Err(unsupported(
                                        sub.span,
                                        "non-constant annotated concat subpattern width",
                                    ));
                                };
                                known += width;
                                pinned.push(Some((sub, width)));
                            }
                            PatKind::Vector(inner) => {
                                known += inner.len() as i128;
                                pinned.push(Some((sub, inner.len() as i128)));
                            }
                            PatKind::Lit(lit) => {
                                let actual = self.literal(lit)?;
                                let actual = self.resolve(&actual)?;
                                let width = self
                                    .vector_len_nexp(&actual, sub.span)
                                    .ok()
                                    .as_ref()
                                    .and_then(crate::nexp::Nexp::as_const);
                                let Some(width) = width else {
                                    return Err(unsupported(
                                        sub.span,
                                        "non-constant concat literal width",
                                    ));
                                };
                                known += width;
                                pinned.push(Some((sub, width)));
                            }
                            _ => {
                                return Err(unsupported(
                                    sub.span,
                                    "vector concat subpattern needs elaboration",
                                ));
                            }
                        }
                    }
                    if wildcards > 1 {
                        return Err(unsupported(
                            pat.span,
                            "multiple unclear widths in vector concat pattern",
                        ));
                    }
                    let rest = total - known;
                    if rest < 0 {
                        return Err(arity(pat.span, total as usize, known as usize));
                    }
                    if wildcards == 0 && rest != 0 {
                        return Err(arity(pat.span, total as usize, known as usize));
                    }
                    for sub in unclear {
                        work.push((sub, slice(rest)));
                    }
                    // The wildcard (if any) takes the remainder `rest` and
                    // binds nothing; literal/vector slices check against
                    // their constant widths.
                    for slot in pinned.into_iter().rev() {
                        match slot {
                            None => {}
                            Some((sub, width)) => match &sub.kind {
                                PatKind::Vector(inner) => {
                                    for element in inner.iter().rev() {
                                        work.push((element, bit.clone()));
                                    }
                                }
                                _ => {
                                    work.push((sub, slice(width)));
                                }
                            },
                        }
                    }
                }
                PatKind::App { func, args } => {
                    // Union-Konstruktor-Applikation (`Some(x)`, `Circle(n)`):
                    // Scheme über `values` holen (Union-Member werden dort
                    // deklariert), gegen den Skrutinee-Typ instanziieren
                    // (quantifizierte Variablen binden, Constraints beweisen)
                    // und Payload-Patterns an die substituierten formalen
                    // Argument-Typen binden. Kein Raten: unbekannte Namen
                    // sind ungebunden, Nicht-Funktionen und Aritäten harte
                    // Fehler, nicht inferierbare Variablen ehrlich
                    // `Unsupported` (aus `instantiate`).
                    let name_text = self.name(func)?;
                    let Some(scheme) = self.program.value(name_text, self.position).cloned() else {
                        return Err(TypeError::unbound_var(pat.span, name_text.to_owned()));
                    };
                    let callee = Type::root(scheme);
                    let TypKind::Fn { args: formals, .. } = &callee.node().kind else {
                        return Err(unsupported(
                            pat.span,
                            format!("pattern constructor '{name_text}' is not a function"),
                        ));
                    };
                    // Sub-patterns align to explicit formals (omitted
                    // `implicit(..)` formals allowed, as at call sites).
                    let explicit: Vec<usize> =
                        Self::align_call(&callee, formals, args.len(), pat.span)?
                            .iter()
                            .enumerate()
                            .filter_map(|(formal, slot)| slot.map(|_| formal))
                            .collect();
                    let callee = if callee.root.typschm.quant.is_empty() {
                        self.compatible(&typ, &callee.child(Step::Result), pat.span)?;
                        callee
                    } else {
                        // Result-only instantiation: payload patterns have
                        // no types yet; the scrutinee binds the result
                        // variables, sub-patterns check against the
                        // substituted formal types below.
                        let callee = self.instantiate_relaxed(callee, &[], Some(&typ), pat.span)?;
                        let result = self.open_existential(&callee.child(Step::Result));
                        self.compatible(&typ, &result, pat.span)?;
                        callee
                    };
                    for (index, pat) in args.iter().enumerate().rev() {
                        work.push((pat, callee.child(Step::Argument(explicit[index]))));
                    }
                }
                _ => {
                    return Err(unsupported(
                        pat.span,
                        "vector, list, disjunctive or dependent pattern binding",
                    ))
                }
            }
        }
        Ok(())
    }
    fn function(&mut self, funcl: &Funcl, measure: Option<(&Pat, &Exp)>) -> TypeResult<()> {
        self.proof_scope = Arc::default();
        let name = self.name(&funcl.name)?.to_owned();
        let scheme = self.program.value(&name, self.position).cloned()
            .ok_or_else(|| unsupported(funcl.span, format!("cannot infer type of unannotated function '{name}' without a val specification")))?;
        self.function_with_type(funcl, measure, Type::root(scheme), None)
    }
    fn function_with_type(
        &mut self,
        funcl: &Funcl,
        measure: Option<(&Pat, &Exp)>,
        typ: Type,
        parent: Option<Arc<Scope>>,
    ) -> TypeResult<()> {
        self.proof_scope = Arc::default();
        self.context = Some(typ.root.clone());
        let TypKind::Fn { args, .. } = &typ.node().kind else {
            return Err(unsupported(
                funcl.span,
                "function has a non-function signature",
            ));
        };
        let (pat, guard, body) = match &funcl.arm.kind {
            PExpKind::PatExp { pat, body } => (pat, None, body),
            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
        };
        let mut scope = Scope {
            parent,
            locals: Vec::new(),
            facts: Vec::new(),
        };
        if args.len() == 1 {
            self.pattern(pat, typ.child(Step::Argument(0)), false, &mut scope)?;
        } else {
            let PatKind::Tuple(items) = &pat.kind else {
                return Err(arity(pat.span, args.len(), 1));
            };
            if args.len() != items.len() {
                return Err(arity(pat.span, args.len(), items.len()));
            }
            for (index, pat) in items.iter().enumerate() {
                self.pattern(pat, typ.child(Step::Argument(index)), false, &mut scope)?;
            }
        }
        let result = typ.child(Step::Result);
        let scope = Arc::new(scope);
        self.proof_scope = scope.clone();
        let body_scope = if let Some(guard) = guard {
            let condition = self.condition(guard, &scope)?;
            scope.branch(condition.as_ref(), false)
        } else {
            scope.clone()
        };
        let mut tasks = vec![Task {
            exp: body,
            expected: result.clone(),
            scope: body_scope,
            returning: result.clone(),
        }];
        if let Some(guard) = guard {
            tasks.push(Task {
                exp: guard,
                expected: self.builtin("bool"),
                scope: scope.clone(),
                returning: result.clone(),
            });
        }
        if let Some((pat, exp)) = measure {
            let args = crate::measure_check::lookup_function_args(
                self.program,
                self.name(&funcl.name)?,
                self.position,
                funcl.span,
            )?;
            let bound = crate::measure_check::expected_pat_type(&args, pat.span);
            let bound_ty = Type::owned(bound, self.it.clone());
            let mut child = Scope {
                locals: Vec::new(),
                facts: Vec::new(),
                parent: Some(scope.clone()),
            };
            self.pattern(pat, bound_ty, false, &mut child)?;
            let int = self.builtin(crate::measure_check::body_builtin());
            tasks.push(Task {
                exp,
                expected: int.clone(),
                scope: Arc::new(child),
                returning: result.clone(),
            });
        }
        self.run(tasks)
    }
    fn measure(&mut self, pat: &Pat, exp: &Exp, args: &[Typ]) -> TypeResult<()> {
        let bound = crate::measure_check::expected_pat_type(args, pat.span);
        let bound_ty = Type::owned(bound, self.it.clone());
        let mut child = Scope {
            locals: Vec::new(),
            facts: Vec::new(),
            parent: None,
        };
        self.pattern(pat, bound_ty, false, &mut child)?;
        let int = self.builtin(crate::measure_check::body_builtin());
        self.run(vec![Task {
            exp,
            expected: int.clone(),
            scope: Arc::new(child),
            returning: int,
        }])
    }
    fn loop_measures(&mut self, measures: &[(Loop, Exp)]) -> TypeResult<()> {
        let int = self.builtin(crate::measure_check::body_builtin());
        let scope = Arc::new(Scope::default());
        let mut tasks = Vec::with_capacity(measures.len());
        for (_, exp) in measures {
            tasks.push(Task {
                exp,
                expected: int.clone(),
                scope: scope.clone(),
                returning: int.clone(),
            });
        }
        self.run(tasks)
    }
    fn condition(&mut self, exp: &Exp, scope: &Scope) -> TypeResult<Option<Type>> {
        match self.infer(exp, scope) {
            Ok(typ) => {
                let boolean = self.builtin("bool");
                self.compatible(&boolean, &typ, exp.span)?;
                // Refined conditions (`bool(..)`, e.g. builtin `lt_int`)
                // already carry their constraint: `branch` assumes it
                // directly, no synthesized duplicate. Opened plain-`bool`
                // witnesses fall through to comparison synthesis, which
                // builds the real constraint from the operands; if there
                // is none, the opened type itself is assumed (its witness
                // links operand views by arc identity).
                if typ.name() == Some("atom_bool") && !is_vacuous_fact(&typ) {
                    return Ok(Some(self.resolve(&typ)?));
                }
                // Comparison guards over atoms also assume their constraint
                // (`m < 'n` gives `'m < 'n`); other conditions assume
                // nothing, as before.
                if let Some(fact) = self.guard_fact(exp, scope, exp.span)? {
                    return Ok(Some(fact));
                }
                Ok(Some(self.resolve(&typ)?))
            }
            // Some expressions can be checked against bool before they can be
            // synthesized. The ordinary task must still check this condition;
            // it contributes no assumptions until its type can be inferred.
            Err(error) if matches!(error.kind, TypeErrorKind::Unsupported(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn infer(&mut self, exp: &Exp, scope: &Scope) -> TypeResult<Type> {
        enum Synthesis<'a> {
            Enter(&'a Exp),
            Call(&'a Exp, Vec<Type>, usize),
            Vector(&'a Exp, usize),
            Tuple(&'a Exp, usize),
            Struct(&'a Exp, usize),
            // LANE-45: infer the base first, then check each update field.
            StructUpdate(&'a Exp, usize),
            VectorAppend(&'a Exp),
            Field(&'a Exp),
            IfCondition(&'a Exp),
            IfBranches(&'a Exp),
            Forward(&'a Exp),
        }
        let mut work = vec![(Synthesis::Enter(exp), Arc::new(scope.clone()))];
        let mut results = Vec::new();
        while let Some((task, scope)) = work.pop() {
            self.proof_scope = scope.clone();
            match task {
                Synthesis::Enter(exp) => {
                    if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
                        results.push(typ.value.clone());
                        continue;
                    }
                    let declared = match &exp.kind {
                        ExpKind::Lit(lit) => self.literal(lit)?,
                        ExpKind::Id(name) => self.id(name, &scope)?,
                        ExpKind::Typ { typ, .. } => self.annotation(typ)?,
                        ExpKind::Block(items) => {
                            if let Some(last) = items.last() {
                                // Asserts before the result extend the scope
                                // (as in checking mode); the full prefix is
                                // checked by the later checking task.
                                let mut current = scope.clone();
                                for item in &items[..items.len() - 1] {
                                    match &item.kind {
                                        ExpKind::Assert { cond, msg } => {
                                            current = self.check_assert(cond, msg, &current)?;
                                        }
                                        ExpKind::Assign { lexp, value } => {
                                            if let Some((_, extended)) =
                                                self.declaring_assignment(item, lexp, value, &current)?
                                            {
                                                current = extended;
                                            }
                                        }
                                        ExpKind::If { cond, then_branch, .. }
                                            if Self::branch_escapes(then_branch) =>
                                        {
                                            let fact = self.condition(cond, &current)?;
                                            current = current.branch(fact.as_ref(), true);
                                        }
                                        _ => {}
                                    }
                                }
                                work.push((Synthesis::Forward(exp), scope.clone()));
                                work.push((Synthesis::Enter(last), current));
                                continue;
                            }
                            self.builtin("unit")
                        }
                        ExpKind::If { cond, .. } => {
                            work.push((Synthesis::IfCondition(exp), scope.clone()));
                            work.push((Synthesis::Enter(cond), scope.clone()));
                            continue;
                        }
                        ExpKind::Tuple(items) => {
                            work.push((Synthesis::Tuple(exp, items.len()), scope.clone()));
                            work.extend(
                                items
                                    .iter()
                                    .rev()
                                    .map(|item| (Synthesis::Enter(item), scope.clone())),
                            );
                            continue;
                        }
                        ExpKind::Vector(items) => {
                            let Some(first) = items.first() else {
                                return Err(unsupported(
                                    exp.span,
                                    "cannot infer the element type of an empty vector",
                                ));
                            };
                            work.push((Synthesis::Vector(exp, items.len()), scope.clone()));
                            work.push((Synthesis::Enter(first), scope.clone()));
                            continue;
                        }
                        ExpKind::VectorAppend(left, right) => {
                            work.push((Synthesis::VectorAppend(exp), scope.clone()));
                            work.push((Synthesis::Enter(right.as_ref()), scope.clone()));
                            work.push((Synthesis::Enter(left.as_ref()), scope.clone()));
                            continue;
                        }
                        ExpKind::Field { record, .. } => {
                            work.push((Synthesis::Field(exp), scope.clone()));
                            work.push((Synthesis::Enter(record.as_ref()), scope.clone()));
                            continue;
                        }
                        ExpKind::Constraint(_) => {
                            let Some(fact) = self.guard_fact(exp, &scope, exp.span)? else {
                                return Err(unsupported(
                                    exp.span,
                                    "constraint expression needs elaboration",
                                ));
                            };
                            results.push(self.value_type(exp, fact));
                            continue;
                        }
                        ExpKind::Sizeof(node) => self.sizeof_type(node, exp.span)?,
                        ExpKind::Struct { name, fields } => {
                            // OCaml `infer_exp` E_struct ~L3774:
                            // `SN_anon` never infers (`StructNameMustMatch`).
                            let rectyp_id = match name {
                                StructName::Anon => {
                                    return Err(unsupported(
                                        exp.span,
                                        "Cannot infer type of struct literal",
                                    ));
                                }
                                StructName::Id(id) => {
                                    let found = self.name(id)?.to_owned();
                                    if self.program.record(&found, self.position).is_none() {
                                        return Err(TypeError::unbound_var(id.span, found));
                                    }
                                    found
                                }
                            };
                            // `FullFieldCoverage`: unknown or duplicate
                            // fields are hard errors; missing fields error.
                            let record = self
                                .program
                                .record(&rectyp_id, self.position)
                                .expect("checked struct record");
                            {
                                let mut seen = std::collections::BTreeSet::new();
                                for fexp in fields.iter() {
                                    let field_name = self.name(&fexp.field)?.to_owned();
                                    if !record.fields.iter().any(|f| f.name == field_name) {
                                        return Err(TypeError::unbound_var(
                                            fexp.field.span,
                                            field_name,
                                        ));
                                    }
                                    if !seen.insert(field_name.clone()) {
                                        return Err(unsupported(
                                            fexp.field.span,
                                            format!("duplicate field '{field_name}'"),
                                        ));
                                    }
                                }
                                if seen.len() != record.fields.len() {
                                    let missing: Vec<_> = record
                                        .fields
                                        .iter()
                                        .map(|f| f.name.clone())
                                        .filter(|n| !seen.contains(n))
                                        .collect();
                                    return Err(unsupported(
                                        exp.span,
                                        format!(
                                            "struct literal missing fields: {}",
                                            missing.join(", ")
                                        ),
                                    ));
                                }
                            }
                            work.push((Synthesis::Struct(exp, fields.len()), scope.clone()));
                            work.extend(
                                fields
                                    .iter()
                                    .rev()
                                    .map(|f| (Synthesis::Enter(&f.value), scope.clone())),
                            );
                            continue;
                        }
                        // LANE-45 infer `E_struct_update` ~L3823: infer the
                        // base; fields are checked in
                        // `Synthesis::StructUpdate`.
                        ExpKind::StructUpdate { record, fields } => {
                            work.push((Synthesis::StructUpdate(exp, fields.len()), scope.clone()));
                            work.push((Synthesis::Enter(record.as_ref()), scope.clone()));
                            continue;
                        }
                        ExpKind::App { func, args } => {
                            if let Some((callee, _)) = self.bitfield_call(exp, &scope, None)? {
                                let result = self.resolve(&callee.child(Step::Result))?;
                                results.push(self.value_type(exp, result));
                                continue;
                            }
                            let candidates = self.call_candidates(func, &scope, args.len())?;
                            self.record_single(exp, &candidates);
                            let args: &[Exp] = Self::effective_args(&candidates, args);
                            // Shallow calls share sibling-guided inference
                            // for both single callees and overload sets.
                            // Deep chains use the iterative worklist to
                            // keep the host stack bounded.
                            if self.synth_depth >= SYNTH_DIRECT_LIMIT {
                                work.push((
                                    Synthesis::Call(exp, candidates, args.len()),
                                    scope.clone(),
                                ));
                                work.extend(
                                    args.iter()
                                        .rev()
                                        .map(|arg| (Synthesis::Enter(arg), scope.clone())),
                                );
                                continue;
                            }
                            self.synth_depth += 1;
                            let actual = self.infer_app_actuals(args, &scope, exp.span);
                            let actual = match actual {
                                Err(error)
                                    if candidates.len() == 1
                                        && matches!(error.kind, TypeErrorKind::Unsupported(_)) =>
                                {
                                    // Arguments that cannot synthesize are
                                    // deferred; the checking pass of this call
                                    // verifies them against the formals.
                                    let deferred = Type::owned(
                                        Node::dummy(TypKind::Unknown),
                                        self.it.clone(),
                                    );
                                    Ok(args
                                        .iter()
                                        .map(|arg| {
                                            self.infer(arg, &scope)
                                                .unwrap_or_else(|_| deferred.clone())
                                        })
                                        .collect())
                                }
                                other => other,
                            };
                            self.synth_depth -= 1;
                            let actual = actual?;
                            self.set_call_hint(func);
                            let callee = self.select_call(candidates, &actual, None, exp.span)?;
                            self.record_call(exp, &callee);
                            self.calls.insert(exp as *const Exp, callee.clone());
                            let result = self.resolve(&callee.child(Step::Result))?;
                            results.push(self.value_type(exp, result));
                            continue;
                        }
                        _ => {
                            return Err(unsupported(
                                exp.span,
                                "synthesis for this expression requires further elaboration",
                            ))
                        }
                    };
                    let declared = self.resolve(&declared)?;
                    results.push(self.value_type(exp, declared));
                }
                Synthesis::IfCondition(exp) => {
                    let condition_type = results.pop().expect("inferred condition");
                    let boolean = self.builtin("bool");
                    self.compatible(&boolean, &condition_type, exp.span)?;
                    let ExpKind::If {
                        cond,
                        then_branch,
                        else_branch,
                    } = &exp.kind
                    else {
                        unreachable!("if synthesis shape")
                    };
                    let condition = self.condition(cond, &scope)?;
                    work.push((Synthesis::IfBranches(exp), scope.clone()));
                    work.push((
                        Synthesis::Enter(else_branch),
                        scope.branch(condition.as_ref(), true),
                    ));
                    work.push((
                        Synthesis::Enter(then_branch),
                        scope.branch(condition.as_ref(), false),
                    ));
                }
                Synthesis::IfBranches(exp) => {
                    let right = results.pop().expect("inferred else branch");
                    let left = results.pop().expect("inferred then branch");
                    let ExpKind::If { then_branch, .. } = &exp.kind else {
                        unreachable!("if synthesis shape")
                    };
                    let declared = (*self
                        .inferred
                        .get(&(then_branch.as_ref() as *const Exp))
                        .expect("inferred then branch")
                        .declared)
                        .clone();
                    let declared = if matches!(left.name(), Some("bool" | "atom_bool")) {
                        self.builtin("bool")
                    } else if matches!(left.name(), Some("int" | "atom"))
                        && matches!(right.name(), Some("int" | "atom"))
                        && self.compatible(&declared, &right, exp.span).is_err()
                    {
                        // Distinct singleton witnesses join into the OCaml
                        // `union_typ` existential `{'j, 'j == a | 'j == b}`;
                        // plain `int` operands just forget the refinement.
                        match self.join_atoms(&left, &right) {
                            Some(joined) => joined,
                            None => self.builtin("int"),
                        }
                    } else {
                        declared
                    };
                    self.compatible(&declared, &right, exp.span)?;
                    results.push(self.value_type(exp, declared));
                }
                Synthesis::Forward(exp) => {
                    results.pop().expect("inferred block result");
                    let ExpKind::Block(items) = &exp.kind else {
                        unreachable!("block synthesis shape")
                    };
                    let last = items.last().expect("nonempty block");
                    // A block denotes its last expression: share that
                    // expression's witness so checking the block against its
                    // own inferred type is the identity.
                    let (value, declared) = {
                        let last = self
                            .inferred
                            .get(&(last as *const Exp))
                            .expect("inferred block result");
                        (last.value.clone(), last.declared.clone())
                    };
                    self.inferred.insert(
                        exp as *const Exp,
                        InferredValue {
                            value: value.clone(),
                            declared,
                        },
                    );
                    results.push(value);
                }
                Synthesis::Call(exp, candidates, count) => {
                    let actual = results.split_off(results.len() - count);
                    if let ExpKind::App { func, .. } = &exp.kind {
                        self.set_call_hint(func);
                    }
                    let callee = self.select_call(candidates, &actual, None, exp.span)?;
                    self.record_call(exp, &callee);
                    let result = self.resolve(&callee.child(Step::Result))?;
                    // value_type inserts the shared declared/value pair; no
                    // second insert here (that would duplicate the entry).
                    let result = self.value_type(exp, result);
                    self.calls.insert(exp as *const Exp, callee);
                    results.push(result);
                }
                Synthesis::VectorAppend(exp) => {
                    let right = results.pop().expect("inferred append right");
                    let left = results.pop().expect("inferred append left");
                    let left_typ = self.resolve(&left)?;
                    let right_typ = self.resolve(&right)?;
                    let result = self.append_result(&left_typ, &right_typ, exp.span)?;
                    let result = self.value_type(exp, result);
                    results.push(result);
                }
                Synthesis::Field(exp) => {
                    let _base = results.pop().expect("inferred field base");
                    let ExpKind::Field { record, field } = &exp.kind else {
                        unreachable!("field synthesis shape")
                    };
                    let declared = self.field_type(record, field, &scope, exp.span)?;
                    let declared = self.resolve(&declared)?;
                    results.push(self.value_type(exp, declared));
                }
                Synthesis::Tuple(exp, count) => {
                    let elements = results.split_off(results.len() - count);
                    let typ = self.tuple_type(elements);
                    let ExpKind::Tuple(items) = &exp.kind else {
                        unreachable!()
                    };
                    // Declared element views are shared via Arc: only a
                    // refcount bump per item, no extra substitution chains.
                    let declared = self.tuple_type(
                        items
                            .iter()
                            .map(|item| {
                                (*self
                                    .inferred
                                    .get(&(item as *const Exp))
                                    .expect("inferred tuple item")
                                    .declared)
                                    .clone()
                            })
                            .collect(),
                    );
                    self.inferred.insert(
                        exp as *const Exp,
                        InferredValue {
                            value: typ.clone(),
                            declared: Arc::new(declared),
                        },
                    );
                    results.push(typ);
                }
                Synthesis::Vector(exp, length) => {
                    results.pop().expect("inferred first vector element");
                    let ExpKind::Vector(items) = &exp.kind else {
                        unreachable!()
                    };
                    // Share the declared element view instead of cloning a
                    // second full value/substitution chain per read.
                    let element = (*self
                        .inferred
                        .get(&(&items[0] as *const Exp))
                        .expect("inferred element")
                        .declared)
                        .clone();
                    let bit = self.refined(
                        "bitvector",
                        TypArgKind::NExp(Node::dummy(NExpKind::Constant("1".into()))),
                    );
                    let typ = if element.name() == Some("bitvector")
                        && self.compatible(&bit, &element, exp.span).is_ok()
                    {
                        self.refined(
                            "bitvector",
                            TypArgKind::NExp(Node::dummy(NExpKind::Constant(length.to_string()))),
                        )
                    } else {
                        self.vector_type(length, element)
                    };
                    results.push(self.value_type(exp, typ));
                }
                Synthesis::Struct(exp, count) => {
                    // OCaml `infer_exp` E_struct ~L3774: every field is
                    // inferred, the telescope is unified field by field
                    // (`merge_uvars`), missing instantiations are hard
                    // errors. `SN_anon` never infers (rejected in `Enter`
                    // and re-checked here).
                    let inferred = results.split_off(results.len() - count);
                    let ExpKind::Struct { name, fields } = &exp.kind else {
                        unreachable!("struct synthesis shape")
                    };
                    let StructName::Id(id) = name else {
                        return Err(unsupported(exp.span, "Cannot infer type of struct literal"));
                    };
                    let rectyp_id = self.name(id)?.to_owned();
                    let record = self
                        .program
                        .record(&rectyp_id, self.position)
                        .cloned()
                        .ok_or_else(|| TypeError::unbound_var(id.span, rectyp_id.clone()))?;
                    let mut quantified: Vec<Symbol> = Vec::new();
                    let mut quantified_scalars: Vec<(Symbol, Kind)> = Vec::new();
                    for item in &record.quant {
                        match item {
                            QuantItem::Id(binder) if binder.kind == Kind::Type => {
                                quantified.push(binder.kid);
                            }
                            QuantItem::Id(binder) => {
                                quantified_scalars.push((binder.kid, binder.kind));
                            }
                            QuantItem::Constraint(_) => {}
                        }
                    }
                    let mut bindings: Vec<(Symbol, Type)> = Vec::new();
                    let mut scalar_bindings: Vec<(Symbol, Scalar)> = Vec::new();
                    for (fexp, inferred_typ) in fields.iter().zip(inferred.iter()) {
                        let field_name = self.name(&fexp.field)?.to_owned();
                        let scheme = self
                            .program
                            .accessor(&rectyp_id, &field_name, self.position)
                            .cloned()
                            .ok_or_else(|| {
                                TypeError::unbound_var(fexp.field.span, field_name.clone())
                            })?;
                        let formal = Type::root(scheme).child(Step::Result);
                        let mut pairs = vec![(formal, inferred_typ.clone())];
                        while let Some((formal, actual)) = pairs.pop() {
                            let formal = self.resolve(&formal)?;
                            let actual = self.resolve(&actual)?;
                            match &formal.node().kind {
                                TypKind::Var(symbol) if quantified.contains(symbol) => {
                                    if let Some((_, bound)) =
                                        bindings.iter().find(|(bound, _)| bound == symbol)
                                    {
                                        self.compatible(bound, &actual, fexp.value.span)?;
                                    } else {
                                        bindings.push((*symbol, actual));
                                    }
                                }
                                TypKind::Tuple(items) => {
                                    let TypKind::Tuple(found) = &actual.node().kind else {
                                        return Err(mismatch(fexp.value.span, &formal, &actual));
                                    };
                                    if items.len() != found.len() {
                                        return Err(arity(
                                            fexp.value.span,
                                            items.len(),
                                            found.len(),
                                        ));
                                    }
                                    for i in 0..items.len() {
                                        pairs.push((
                                            formal.child(Step::Tuple(i)),
                                            actual.child(Step::Tuple(i)),
                                        ));
                                    }
                                }
                                TypKind::App { args, .. } => {
                                    let TypKind::App { args: found, .. } = &actual.node().kind
                                    else {
                                        return Err(mismatch(fexp.value.span, &formal, &actual));
                                    };
                                    if formal.name() != actual.name() {
                                        return Err(mismatch(fexp.value.span, &formal, &actual));
                                    }
                                    if args.len() != found.len() {
                                        return Err(arity(
                                            fexp.value.span,
                                            args.len(),
                                            found.len(),
                                        ));
                                    }
                                    for (i, (arg, _)) in args.iter().zip(found).enumerate() {
                                        match (&arg.kind, &found[i].kind) {
                                            (TypArgKind::Typ(_), TypArgKind::Typ(_)) => {
                                                pairs.push((
                                                    formal.child(Step::TypeArgument(i)),
                                                    actual.child(Step::TypeArgument(i)),
                                                ));
                                            }
                                            (
                                                TypArgKind::NExp(_) | TypArgKind::Bool(_),
                                                TypArgKind::NExp(_) | TypArgKind::Bool(_),
                                            ) => {
                                                let formal_arg = Scalar {
                                                    owner: formal.clone(),
                                                    index: i,
                                                }
                                                .resolved();
                                                let actual_arg = Scalar {
                                                    owner: actual.clone(),
                                                    index: i,
                                                }
                                                .resolved();
                                                let variable = match &formal_arg.argument().kind {
                                                    TypArgKind::NExp(node) => match &node.kind {
                                                        NExpKind::Var(k) => Some((*k, Kind::Int)),
                                                        _ => None,
                                                    },
                                                    TypArgKind::Bool(c) => match &c.kind {
                                                        ConstraintKind::Var(k) => {
                                                            Some((*k, Kind::Bool))
                                                        }
                                                        _ => None,
                                                    },
                                                    _ => None,
                                                };
                                                if let Some((symbol, _)) =
                                                    variable.filter(|(s, k)| {
                                                        quantified_scalars.iter().any(
                                                            |(bound, bound_kind)| {
                                                                bound == s && bound_kind == k
                                                            },
                                                        )
                                                    })
                                                {
                                                    if let Some((_, bound)) = scalar_bindings
                                                        .iter()
                                                        .find(|(bound, _)| *bound == symbol)
                                                    {
                                                        self.scalar_equal(
                                                            bound,
                                                            &actual_arg,
                                                            fexp.value.span,
                                                        )?;
                                                    } else {
                                                        scalar_bindings.push((symbol, actual_arg));
                                                    }
                                                } else {
                                                    self.scalar_equal(
                                                        &formal_arg,
                                                        &actual_arg,
                                                        fexp.value.span,
                                                    )?;
                                                }
                                            }
                                            _ => {
                                                return Err(mismatch(
                                                    fexp.value.span,
                                                    &formal,
                                                    &actual,
                                                ));
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    self.compatible(&formal, &actual, fexp.value.span)?;
                                }
                            }
                        }
                    }
                    for item in &record.quant {
                        if let QuantItem::Id(binder) = item {
                            let bound = if binder.kind == Kind::Type {
                                bindings.iter().any(|(b, _)| *b == binder.kid)
                            } else {
                                scalar_bindings.iter().any(|(b, _)| *b == binder.kid)
                            };
                            if !bound {
                                return Err(unsupported(
                                    exp.span,
                                    format!(
                                        "Failed to instantiate {} when inferring struct literal of type {}",
                                        self.it.resolve(binder.kid).unwrap_or("<unknown>"),
                                        rectyp_id
                                    ),
                                ));
                            }
                        }
                    }
                    // Apply the merged telescope bindings to the record
                    // argument. The declaration interner still owns the
                    // telescope symbols, so the instantiated head resolves
                    // without fresh-symbol bookkeeping.
                    let mut interner = (*record.interner).clone();
                    let rec_name = Name::dummy(interner.intern(&rectyp_id));
                    let mut args = Vec::with_capacity(record.quant.len());
                    for item in &record.quant {
                        let QuantItem::Id(binder) = item else {
                            continue;
                        };
                        match binder.kind {
                            Kind::Type => {
                                args.push(Node::dummy(TypArgKind::Typ(Node::dummy(TypKind::Var(
                                    binder.kid,
                                )))));
                            }
                            Kind::Int => {
                                args.push(Node::dummy(TypArgKind::NExp(Node::dummy(
                                    NExpKind::Var(binder.kid),
                                ))));
                            }
                            Kind::Bool => {
                                args.push(Node::dummy(TypArgKind::Bool(Box::new(Node::dummy(
                                    ConstraintKind::Var(binder.kid),
                                )))));
                            }
                        }
                    }
                    let mut record_typ = Type::owned(
                        Node::dummy(if args.is_empty() {
                            TypKind::Id(rec_name)
                        } else {
                            TypKind::App {
                                name: rec_name,
                                args,
                            }
                        }),
                        Arc::new(interner),
                    );
                    record_typ.substitutions = Arc::new(Substitutions {
                        types: bindings,
                        scalars: scalar_bindings,
                    });
                    let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
                    let assumptions = self.proof_assumptions(&scope);
                    for q in &record.quant {
                        if let QuantItem::Constraint(c) = q {
                            let query = CanonicalQuery::implication(
                                &assumptions,
                                View {
                                    negated: false,
                                    constraint: c,
                                    context: record_typ.proof_context(),
                                },
                                None,
                            )
                            .map_err(|e| unsupported(exp.span, e))?;
                            if !self.program.prove(query).is_valid() {
                                return Err(TypeError::constraint_not_proved(
                                    exp.span,
                                    "dependent struct quantifier constraint",
                                ));
                            }
                        }
                    }
                    let record_typ = self.resolve(&record_typ)?;
                    results.push(self.value_type(exp, record_typ));
                }
                Synthesis::StructUpdate(exp, _count) => {
                    // OCaml `infer_exp` E_struct_update ~L3823: the base was
                    // inferred first; require a declared (non-generic)
                    // record, then check each update field against its
                    // accessor payload unified with the inferred base.
                    let base = results.pop().expect("inferred struct update base");
                    let base = self.resolve(&base)?;
                    let record_name = base
                        .name()
                        .map(str::to_owned)
                        .filter(|name| self.program.record(name, self.position).is_some());
                    let Some(record_name) = record_name else {
                        let label = base.label();
                        return Err(TypeError {
                            span: exp.span,
                            kind: TypeErrorKind::QuantError(format!(
                                "The type {label} is not a record"
                            )),
                        });
                    };
                    let ExpKind::StructUpdate { fields, .. } = &exp.kind else {
                        unreachable!("struct update synthesis shape")
                    };
                    let record_def = self
                        .program
                        .record(&record_name, self.position)
                        .expect("checked record");
                    if !record_def.quant.is_empty() {
                        return Err(unsupported(
                            exp.span,
                            "generic struct update requires elaboration",
                        ));
                    }
                    for fexp in fields.iter() {
                        let field_name = self.name(&fexp.field)?;
                        let field_typ =
                            self.struct_field_type(&record_name, field_name, &base, fexp.span)?;
                        let inferred = self.infer(&fexp.value, &scope)?;
                        self.compatible(&field_typ, &inferred, fexp.span)?;
                    }
                    results.push(self.value_type(exp, base));
                }
            }
        }
        Ok(results.pop().expect("synthesized root"))
    }

    fn instantiate(
        &self,
        callee: Type,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
    ) -> TypeResult<Type> {
        self.instantiate_inner(callee, actual, expected, span, false)
    }

    /// Result-only instantiation for patterns: payload sub-patterns carry
    /// no types yet, so no arity is enforced here (the caller aligns
    /// sub-patterns to explicit formals itself); the scrutinee binds the
    /// result variables through the regular checks below.
    fn instantiate_relaxed(
        &self,
        callee: Type,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
    ) -> TypeResult<Type> {
        self.instantiate_inner(callee, actual, expected, span, true)
    }

    fn instantiate_inner(
        &self,
        mut callee: Type,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
        relaxed_arity: bool,
    ) -> TypeResult<Type> {
        // Compact vectors: most calls quantify zero to a handful of
        // variables; tree nodes per call retain megabytes in deep programs.
        let mut quantified: Vec<Symbol> = Vec::new();
        let mut quantified_scalars: Vec<(Symbol, Kind)> = Vec::new();
        for item in &callee.root.typschm.quant {
            match item {
                QuantItem::Id(binder) if binder.kind == Kind::Type => {
                    quantified.push(binder.kid);
                }
                QuantItem::Id(binder) => {
                    quantified_scalars.push((binder.kid, binder.kind));
                }
                QuantItem::Constraint(_) => {}
            }
        }
        let mut bindings = callee.substitutions.types.clone();
        let mut scalar_bindings = callee.substitutions.scalars.clone();
        let mut pairs = Vec::with_capacity(actual.len() + 1);
        // Omitted `implicit(..)` formals consume no actual (aligned by
        // `align_call`: same deterministic mapping as the callers).
        // Relaxed (patterns): sub-patterns carry no types yet; the
        // caller pre-checked their arity itself.
        if relaxed_arity {
            for (index, typ) in actual.iter().enumerate() {
                pairs.push((callee.child(Step::Argument(index)), typ.clone(), false));
            }
        } else {
            let formal_arity = match &callee.node().kind {
                TypKind::Fn { args, .. } => args.clone(),
                _ => return Err(unsupported(span, "application of non-function")),
            };
            let alignment = Self::align_call(&callee, &formal_arity, actual.len(), span)?;
            for (formal_index, slot) in alignment.iter().enumerate() {
                if let Some(actual_index) = slot {
                    pairs.push((
                        callee.child(Step::Argument(formal_index)),
                        actual[*actual_index].clone(),
                        false,
                    ));
                }
            }
        }
        if let Some(expected) = expected {
            pairs.push((callee.child(Step::Result), expected.clone(), true));
        }
        while let Some((formal, actual, result_context)) = pairs.pop() {
            // Deferred argument: checked against the instantiated formal later.
            if matches!(actual.node().kind, TypKind::Unknown) {
                continue;
            }
            let formal = self.resolve(&formal)?;
            let actual = self.resolve(&actual)?;
            if result_context
                && (matches!(formal.node().kind, TypKind::Exist { .. })
                    || matches!(actual.node().kind, TypKind::Exist { .. }))
            {
                // Local existential binders are witnesses, not the callee's
                // quantified parameters. Bind parameters from arguments first;
                // select_call checks result subsumption after substitutions.
                // A parameter inferable only through this existential still
                // fails the unresolved-parameter check below.
                continue;
            }
            match &formal.node().kind {
                TypKind::Var(symbol)
                    if Arc::ptr_eq(&formal.root, &callee.root) && quantified.contains(symbol) =>
                {
                    if let Some((_, bound)) = bindings.iter().find(|(bound, _)| bound == symbol) {
                        self.compatible(bound, &actual, span)?;
                    } else {
                        bindings.push((*symbol, actual));
                    }
                }
                TypKind::Tuple(items) => {
                    let TypKind::Tuple(found) = &actual.node().kind else {
                        return Err(mismatch(span, &formal, &actual));
                    };
                    if items.len() != found.len() {
                        return Err(arity(span, items.len(), found.len()));
                    }
                    for i in 0..items.len() {
                        pairs.push((
                            formal.child(Step::Tuple(i)),
                            actual.child(Step::Tuple(i)),
                            result_context,
                        ));
                    }
                }
                TypKind::App { args, .. } => {
                    if matches!(
                        (&actual.node().kind, actual.name(), formal.name()),
                        (TypKind::Id(_), Some("int"), Some("atom"))
                            | (TypKind::Id(_), Some("bool"), Some("atom_bool"))
                    ) {
                        // A plain expected return type forgets the refinement.
                        // Plain arguments still need existential opening.
                        if !result_context {
                            return Err(unsupported(
                                span,
                                "opening an unrefined integer/Boolean call argument",
                            ));
                        }
                        self.compatible(&actual, &formal, span)?;
                        continue;
                    }
                    let TypKind::App { args: found, .. } = &actual.node().kind else {
                        return Err(unsupported(span, format!("synonym expansion during instantiation (formal {}, actual {}, result_context {})", formal.label(), actual.label(), result_context)));
                    };
                    if formal.name() != actual.name() {
                        return Err(mismatch(span, &formal, &actual));
                    }
                    if args.len() != found.len() {
                        return Err(arity(span, args.len(), found.len()));
                    }
                    for (i, (arg, found)) in args.iter().zip(found).enumerate() {
                        match (&arg.kind, &found.kind) {
                            (TypArgKind::Typ(_), TypArgKind::Typ(_)) => pairs.push((
                                formal.child(Step::TypeArgument(i)),
                                actual.child(Step::TypeArgument(i)),
                                result_context,
                            )),
                            (TypArgKind::NExp(_), TypArgKind::NExp(_))
                            | (TypArgKind::Bool(_), TypArgKind::Bool(_)) => {
                                let formal_arg = Scalar {
                                    owner: formal.clone(),
                                    index: i,
                                }
                                .resolved();
                                let actual_arg = Scalar {
                                    owner: actual.clone(),
                                    index: i,
                                }
                                .resolved();
                                let variable = match &formal_arg.argument().kind {
                                    TypArgKind::NExp(Node {
                                        kind: NExpKind::Var(k),
                                        ..
                                    }) => Some((*k, Kind::Int)),
                                    TypArgKind::Bool(c) => {
                                        if let ConstraintKind::Var(k) = c.kind {
                                            Some((k, Kind::Bool))
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                };
                                if let Some((symbol, kind)) = variable.filter(|(s, k)| {
                                    Arc::ptr_eq(&formal_arg.owner.root, &callee.root)
                                        && quantified_scalars.iter().any(|(bound, bound_kind)| {
                                            bound == s && bound_kind == k
                                        })
                                }) {
                                    let _ = kind;
                                    if !scalar_bindings.iter().any(|(bound, _)| *bound == symbol) {
                                        scalar_bindings.push((symbol, actual_arg));
                                    }
                                }
                            }
                            _ => return Err(mismatch(span, &formal, &actual)),
                        }
                    }
                }
                _ => self.compatible(&formal, &actual, span)?,
            }
        }
        // Unused quantifiers need no binding. Variables that occur in the
        // result must be fixed before the result can escape this call.
        let mut work = vec![callee.child(Step::Result)];
        while let Some(typ) = work.pop() {
            match &typ.node().kind {
                TypKind::Var(symbol)
                    if quantified.contains(symbol)
                        && !bindings.iter().any(|(bound, _)| bound == symbol) =>
                {
                    return Err(unsupported(
                        span,
                        "cannot infer polymorphic result variable",
                    ))
                }
                TypKind::Tuple(items) => {
                    work.extend((0..items.len()).map(|i| typ.child(Step::Tuple(i))))
                }
                TypKind::App { args, .. } => {
                    work.extend(args.iter().enumerate().filter_map(|(i, arg)| {
                        matches!(arg.kind, TypArgKind::Typ(_))
                            .then(|| typ.child(Step::TypeArgument(i)))
                    }))
                }
                _ => {}
            }
        }
        if quantified_scalars
            .iter()
            .any(|(symbol, _)| !scalar_bindings.iter().any(|(bound, _)| bound == symbol))
        {
            return Err(unsupported(span,"cannot infer dependent call parameter (existential opening or arithmetic solving required)"));
        }
        callee.substitutions = Arc::new(Substitutions {
            types: bindings,
            scalars: scalar_bindings,
        });
        // select_call checks every argument and the return after installing
        // all substitutions. Re-expand aliases there: this composes alias
        // binders with fresh call binders without mutating borrowed contexts.
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let mut assumptions = self.proof_assumptions(&scope);
        // Dropped existential bounds re-enter here with the opening's
        // identities (e.g. `uint` guarantees `0 <= 'm` for `mod`).
        // `actual` borrows outlive the queries below.
        let mut actual_refs: Vec<&Type> = actual.iter().collect();
        // Relaxed instantiation binds from an existing scrutinee/witness.
        // Only in that mode is the result-side type established evidence.
        // Ordinary call result expectations must never become assumptions.
        if relaxed_arity {
            actual_refs.extend(expected);
        }
        assumptions.extend(self.marker_assumptions(&actual_refs));
        assumptions.extend(self.marker_assumptions(&self.fact_owners()));
        for q in &callee.root.typschm.quant {
            if let QuantItem::Constraint(c) = q {
                let query = CanonicalQuery::implication(
                    &assumptions,
                    View {
                        negated: false,
                        constraint: c,
                        context: callee.proof_context(),
                    },
                    None,
                )
                .map_err(|e| unsupported(span, e))?;
                if !self.program.prove(query).is_valid() {
                    return Err(TypeError::constraint_not_proved(
                        span,
                        "dependent call quantifier constraint",
                    ));
                }
            }
        }
        Ok(callee)
    }
    fn run(&mut self, mut tasks: Vec<Task<'_>>) -> TypeResult<()> {
        while let Some(Task {
            exp,
            expected,
            scope,
            returning,
        }) = tasks.pop()
        {
            self.proof_scope = scope.clone();
            self.expressions += 1;
            let mut expected = self.resolve(&expected)?;
            if self.recorder.is_some() {
                let recorded = match self.inferred.get(&(exp as *const Exp)) {
                    Some(inferred) => (*inferred.declared).clone(),
                    None => expected.clone(),
                };
                self.record_exp(exp, &recorded);
            }
            // Synthesis opens one witness for the result of the whole
            // control expression. Its children check against the declared
            // result type, not against that fresh result's singleton identity.
            if matches!(exp.kind, ExpKind::If { .. } | ExpKind::Block(_)) {
                if let Some(inferred) = self.inferred.get(&(exp as *const Exp)) {
                    self.compatible(&expected, &inferred.value, exp.span)?;
                    expected = self.resolve(&inferred.declared)?;
                }
            }
            match &exp.kind {
                ExpKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::Id(name) => {
                    let actual = self.resolve(&self.id(name, &scope)?)?;
                    let actual = self.value_type(exp, actual);
                    self.compatible(&expected, &actual, exp.span)?;
                }
                ExpKind::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    let condition = self.condition(cond, &scope)?;
                    tasks.push(Task {
                        exp: else_branch,
                        expected: expected.clone(),
                        scope: scope.branch(condition.as_ref(), true),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: then_branch,
                        expected,
                        scope: scope.branch(condition.as_ref(), false),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: cond,
                        expected: self.builtin("bool"),
                        scope,
                        returning,
                    });
                }
                ExpKind::Block(items) => {
                    if items.is_empty() {
                        let unit = self.builtin("unit");
                        self.compatible(&expected, &unit, exp.span)?;
                    }
                    // Asserts extend the scope for following statements
                    // (OCaml `check_block` facts); anything else queues
                    // unchanged. Asserts check eagerly here, so their
                    // conditions elaborate once (memoized).
                    let mut pending = Vec::with_capacity(items.len());
                    let mut current = scope.clone();
                    for (index, item) in items.iter().enumerate() {
                        let expected = if index + 1 == items.len() {
                            expected.clone()
                        } else {
                            self.builtin("unit")
                        };
                        if let ExpKind::Assert { cond, msg } = &item.kind {
                            let unit = self.builtin("unit");
                            self.compatible(&expected, &unit, item.span)?;
                            self.record_exp(item, &unit);
                            current = self.check_assert(cond, msg, &current)?;
                        } else if let ExpKind::If { cond, then_branch, .. } = &item.kind {
                            if index + 1 < items.len() && Self::branch_escapes(then_branch) {
                                let fact = self.condition(cond, &current)?;
                                pending.push((item, expected, current.clone()));
                                current = current.branch(fact.as_ref(), true);
                            } else {
                                pending.push((item, expected, current.clone()));
                            }
                        } else if let ExpKind::Assign { lexp, value } = &item.kind {
                            if let Some((bound, extended)) =
                                self.declaring_assignment(item, lexp, value, &current)?
                            {
                                let unit = self.builtin("unit");
                                self.compatible(&expected, &unit, item.span)?;
                                self.record_exp(item, &unit);
                                pending.push((
                                    value.as_ref(),
                                    bound,
                                    current.clone(),
                                ));
                                current = extended;
                            } else {
                                pending.push((item, expected, current.clone()));
                            }
                        } else {
                            pending.push((item, expected, current.clone()));
                        }
                    }
                    for (item, expected, item_scope) in pending.into_iter().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected,
                            scope: item_scope,
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Typ { typ, exp: child } => {
                    let annotated = self.annotation(typ)?;
                    let value = self.resolve(&annotated)?;
                    let value = self.value_type(exp, value);
                    self.compatible(&expected, &value, exp.span)?;
                    tasks.push(Task {
                        exp: child,
                        expected: annotated,
                        scope,
                        returning,
                    });
                }
                ExpKind::App { func, args } => {
                    if let Some((callee, kept)) = self.bitfield_call(exp, &scope, Some(&expected))? {
                        let result = self.resolve(&callee.child(Step::Result))?;
                        let result = self.value_type(exp, result);
                        self.compatible(&expected, &result, exp.span)?;
                        for (formal, source) in kept.iter().enumerate().rev() {
                            tasks.push(Task {
                                exp: &args[*source],
                                expected: callee.child(Step::Argument(formal)),
                                scope: scope.clone(),
                                returning: returning.clone(),
                            });
                        }
                        continue;
                    }
                    let mut candidates = self.call_candidates(func, &scope, args.len())?;
                    self.record_single(exp, &candidates);
                    let args: &[Exp] = Self::effective_args(&candidates, args);
                    let callee = if let Some(callee) = self.calls.get(&(exp as *const Exp)) {
                        callee.clone()
                    } else if candidates.len() != 1 || !candidates[0].root.typschm.quant.is_empty()
                    {
                        let single = candidates.len() == 1;
                        let actual = match self.infer_app_actuals(args, &scope, exp.span) {
                            Ok(actual) => actual,
                            // A single callee can still be instantiated from
                            // the arguments that synthesize and the expected
                            // result; the others are checked afterwards
                            // against their instantiated formals.
                            Err(error) if single && matches!(error.kind, TypeErrorKind::Unsupported(_)) => {
                                let deferred = Type::owned(
                                    Node::dummy(TypKind::Unknown),
                                    self.it.clone(),
                                );
                                args.iter()
                                    .map(|arg| {
                                        self.infer(arg, &scope).unwrap_or_else(|_| deferred.clone())
                                    })
                                    .collect()
                            }
                            Err(error) => return Err(error),
                        };
                        self.set_call_hint(func);
                        let selected = self.select_call(candidates, &actual, Some(&expected), exp.span)?;
                        self.record_call(exp, &selected);
                        selected
                    } else {
                        candidates.pop().unwrap()
                    };
                    let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                        return Err(unsupported(exp.span, "application of non-function"));
                    };
                    let explicit: Vec<usize> =
                        Self::align_call(&callee, formal, args.len(), exp.span)?
                            .iter()
                            .enumerate()
                            .filter_map(|(formal, slot)| slot.map(|_| formal))
                            .collect();
                    let result = self.resolve(&callee.child(Step::Result))?;
                    let result = self.value_type(exp, result);
                    self.compatible(&expected, &result, exp.span)?;
                    for (index, arg) in args.iter().enumerate().rev() {
                        tasks.push(Task {
                            exp: arg,
                            expected: callee.child(Step::Argument(explicit[index])),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Tuple(items) => {
                    let TypKind::Tuple(types) = &expected.node().kind else {
                        return Err(unsupported(
                            exp.span,
                            "tuple expression against a type synonym",
                        ));
                    };
                    if items.len() != types.len() {
                        return Err(arity(exp.span, types.len(), items.len()));
                    }
                    for (index, item) in items.iter().enumerate().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected: expected.child(Step::Tuple(index)),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                // LANE-45-STRUCTUPDATE-CHECK-ANCHOR (disjoint from Lane-44
                // `E_struct` check anchor at the trailing `_ =>` fallback):
                // this arm sits between `Tuple` and `Let`, so the two
                // patches never share context lines. OCaml `check_exp
                // E_struct_update ~L2199`: the base record checks against
                // the expected record type FIRST (`<= Expected`), then each
                // update field checks against its accessor payload
                // (`RecordField{Expected}`). NO coverage obligation:
                // partial updates (and empty updates) are legal.
                ExpKind::StructUpdate { record, fields } => {
                    let record_name = expected.name().map(|name| name.to_owned()).filter(|_| {
                        expected
                            .name()
                            .is_some_and(|name| self.program.record(name, self.position).is_some())
                    });
                    let Some(record_name) = record_name else {
                        let label = expected.label();
                        return Err(TypeError {
                            span: exp.span,
                            kind: TypeErrorKind::QuantError(format!(
                                "The type {label} is not a record"
                            )),
                        });
                    };
                    let record_def = self
                        .program
                        .record(&record_name, self.position)
                        .expect("checked record");
                    if !record_def.quant.is_empty() {
                        return Err(unsupported(
                            exp.span,
                            "generic struct update requires elaboration",
                        ));
                    }
                    for fexp in fields.iter().rev() {
                        let field_name = self.name(&fexp.field)?;
                        let field_typ =
                            self.struct_field_type(&record_name, field_name, &expected, fexp.span)?;
                        tasks.push(Task {
                            exp: &fexp.value,
                            expected: field_typ,
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                    tasks.push(Task {
                        exp: record.as_ref(),
                        expected: expected.clone(),
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                }
                ExpKind::Field { record, field } => {
                    let field_typ = self.field_type(record, field, &scope, exp.span)?;
                    let result = self.value_type(exp, field_typ);
                    self.compatible(&expected, &result, exp.span)?;
                }
                ExpKind::Let { pat, bind, body } => {
                    // An ascribed pattern guides inference (OCaml checks
                    // the bound against the annotation): names bind the
                    // annotated type, and the bound checks against it.
                    // Unascribed patterns bind the inferred type as before.
                    let bound = if let PatKind::Typ { typ, .. } = &pat.kind {
                        self.annotation(typ)?
                    } else {
                        self.infer(bind, &scope)?
                    };
                    let mut child = Scope {
                        locals: Vec::new(),
                        facts: Vec::new(),
                        parent: Some(scope.clone()),
                    };
                    self.pattern(pat, bound.clone(), false, &mut child)?;
                    tasks.push(Task {
                        exp: body,
                        expected,
                        scope: Arc::new(child),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: bind,
                        expected: bound,
                        scope,
                        returning,
                    });
                }
                ExpKind::Var { lexp, init, body } => {
                    // OCaml bind_assignment: an unannotated mutable keeps the
                    // inferred type (no widening to int); an annotated one
                    // keeps the annotation. The declared init view is shared
                    // via Arc; opening for proofs must not narrow it.
                    let bound = if let LExpKind::Typ { typ, .. } = &lexp.kind {
                        self.annotation(typ)?
                    } else {
                        self.infer(init, &scope)?;
                        (*self
                            .inferred
                            .get(&(init.as_ref() as *const Exp))
                            .expect("inferred initializer")
                            .declared)
                            .clone()
                    };
                    let child = self.mutable_scope(lexp, bound.clone(), &scope)?;
                    tasks.push(Task {
                        exp: body,
                        expected,
                        scope: Arc::new(child),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: init,
                        expected: bound,
                        scope,
                        returning,
                    });
                }
                ExpKind::Assign { lexp, value } => {
                    let unit = self.builtin("unit");
                    self.compatible(&expected, &unit, exp.span)?;
                    let target = self.assignment_type(lexp, &scope)?;
                    tasks.push(Task {
                        exp: value,
                        expected: target,
                        scope,
                        returning,
                    });
                }
                ExpKind::Loop {
                    loop_kind,
                    measure,
                    cond,
                    body,
                } => {
                    let unit = self.builtin("unit");
                    self.compatible(&expected, &unit, exp.span)?;
                    let condition = self.condition(cond, &scope)?;
                    // While may assume the positive condition in the body;
                    // repeat/until must not assume its later condition yet.
                    let body_scope = if *loop_kind == Loop::While {
                        scope.branch(condition.as_ref(), false)
                    } else {
                        scope.clone()
                    };
                    tasks.push(Task {
                        exp: body,
                        expected: unit,
                        scope: body_scope,
                        returning: returning.clone(),
                    });
                    if let Some(measure) = measure {
                        tasks.push(Task {
                            exp: measure,
                            expected: self.builtin("int"),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                    tasks.push(Task {
                        exp: cond,
                        expected: self.builtin("bool"),
                        scope,
                        returning,
                    });
                }
                ExpKind::For {
                    var,
                    from,
                    to,
                    step,
                    order,
                    body,
                } => {
                    let unit = self.builtin("unit");
                    self.compatible(&expected, &unit, exp.span)?;
                    let from_type = self.infer(from, &scope)?;
                    let to_type = self.infer(to, &scope)?;
                    let (lower, upper) = if *order == Order::Inc {
                        (from_type, to_type)
                    } else {
                        (to_type, from_type)
                    };
                    let body_scope = self.iteration_scope(var, lower, upper, &scope)?;
                    tasks.push(Task {
                        exp: body,
                        expected: unit,
                        scope: body_scope,
                        returning: returning.clone(),
                    });
                    let integer = self.builtin("int");
                    for bound in [step, to, from] {
                        tasks.push(Task {
                            exp: bound,
                            expected: integer.clone(),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Try { exp: tried, arms } => {
                    // OCaml check_exp E_try ~L2196 (check-only): the body
                    // checks against the expected type; every arm binds
                    // against exc_typ (`exception`, repo shorthand "exc")
                    // exactly like a match arm (check_case env exc_typ).
                    // No infer plan: synthesis keeps failing as unsupported.
                    tasks.push(Task {
                        exp: tried,
                        expected: expected.clone(),
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                    for arm in arms.iter().rev() {
                        let (pat, guard, body) = match &arm.kind {
                            PExpKind::PatExp { pat, body } => (pat, None, body),
                            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
                        };
                        let mut child = Scope {
                            locals: Vec::new(),
                            facts: Vec::new(),
                            parent: Some(scope.clone()),
                        };
                        // Repo shorthand "exc" = OCaml exc_typ `exception`.
                        let exc = self.builtin("exception");
                        self.pattern(pat, exc, false, &mut child)?;
                        let child = Arc::new(child);
                        let body_scope = if let Some(guard_exp) = guard {
                            let condition = self.condition(guard_exp, &child)?;
                            child.branch(condition.as_ref(), false)
                        } else {
                            child.clone()
                        };
                        tasks.push(Task {
                            exp: body,
                            expected: expected.clone(),
                            scope: body_scope,
                            returning: returning.clone(),
                        });
                        if let Some(guard_exp) = guard {
                            tasks.push(Task {
                                exp: guard_exp,
                                expected: self.builtin("bool"),
                                scope: child,
                                returning: returning.clone(),
                            });
                        }
                    }
                }
                ExpKind::Return(child) => {
                    if self.context.is_none() {
                        return Err(TypeError {
                            span: exp.span,
                            kind: TypeErrorKind::QuantError("return outside a function".into()),
                        });
                    }
                    tasks.push(Task {
                        exp: child,
                        expected: returning.clone(),
                        scope,
                        returning,
                    });
                }
                ExpKind::Throw(payload) => {
                    // OCaml `check_exp` `E_throw`: the payload elaborates
                    // against the `exception` union; the expression
                    // diverges, so it checks against any expected type.
                    // Without a declared `exception` union there is nothing
                    // to check membership against (honest pass-through).
                    let inferred = self.infer(payload, &scope)?;
                    let base = self.resolve(&inferred)?;
                    if self.program.variant("exception", self.position).is_some()
                        && base.name() != Some("exception")
                    {
                        return Err(TypeError::mismatch(
                            exp.span,
                            "exception".to_owned(),
                            base.label(),
                        ));
                    }
                }
                ExpKind::Assert { cond, msg } => {
                    let unit = self.builtin("unit");
                    self.compatible(&expected, &unit, exp.span)?;
                    let _ = self.check_assert(cond, msg, &scope)?;
                }
                ExpKind::Constraint(_) => {
                    // OCaml `infer_exp` `E_constraint`: well-formedness is
                    // the rigid-scope check; the expression denotes
                    // `atom_bool` of the constraint (usable as a fact
                    // source via `guard_fact`).
                    let Some(fact) = self.guard_fact(exp, &scope, exp.span)? else {
                        return Err(unsupported(
                            exp.span,
                            "constraint expression needs elaboration",
                        ));
                    };
                    let result = self.value_type(exp, fact);
                    self.compatible(&expected, &result, exp.span)?;
                }
                ExpKind::Exit(argument) => {
                    // OCaml `check_exp` `E_exit`: the argument checks
                    // against `unit`; the expression diverges.
                    tasks.push(Task {
                        exp: argument,
                        expected: self.builtin("unit"),
                        scope,
                        returning,
                    });
                }
                ExpKind::Undef => {}
                ExpKind::Sizeof(_) => {
                    let result = self.infer(exp, &scope)?;
                    self.compatible(&expected, &result, exp.span)?;
                }
                ExpKind::List(items) if expected.name() == Some("list") => {
                    let TypKind::App { args, .. } = &expected.node().kind else {
                        return Err(unsupported(exp.span, "list type arguments"));
                    };
                    if !matches!(args.first().map(|arg| &arg.kind), Some(TypArgKind::Typ(_))) {
                        return Err(unsupported(exp.span, "list element type"));
                    }
                    for item in items.iter().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected: expected.child(Step::TypeArgument(0)),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Ref(name) => {
                    // OCaml `infer_exp` E_ref: a register reference has type
                    // `register(T)` for the register's declared type `T`.
                    let text = self.name(name)?;
                    let Some(register) = self.program.register(text, self.position) else {
                        return Err(TypeError::unbound_var(name.span, text.to_owned()));
                    };
                    let mut interner = (*register.interner).clone();
                    let head = Name::dummy(interner.intern("register"));
                    let typ = Node::dummy(TypKind::App {
                        name: head,
                        args: vec![Node::dummy(TypArgKind::Typ(register.typschm.typ.clone()))],
                    });
                    let found = Type::root(Arc::new(Scheme {
                        typschm: TypSchm {
                            quant: Vec::new(),
                            typ,
                        },
                        interner: Arc::new(interner),
                    }));
                    self.compatible(&expected, &found, exp.span)?;
                }
                ExpKind::Cons(head, tail) if expected.name() == Some("list") => {
                    let TypKind::App { args, .. } = &expected.node().kind else {
                        return Err(unsupported(exp.span, "list type arguments"));
                    };
                    if !matches!(args.first().map(|arg| &arg.kind), Some(TypArgKind::Typ(_))) {
                        return Err(unsupported(exp.span, "list element type"));
                    }
                    tasks.push(Task {
                        exp: tail,
                        expected: expected.clone(),
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                    tasks.push(Task {
                        exp: head,
                        expected: expected.child(Step::TypeArgument(0)),
                        scope: scope.clone(),
                        returning: returning.clone(),
                    });
                }
                ExpKind::Vector(items) => {
                    let element = match expected.name() {
                        Some("bitvector") => self.refined(
                            "bitvector",
                            TypArgKind::NExp(Node::dummy(NExpKind::Constant("1".into()))),
                        ),
                        Some("vector") => expected.child(Step::TypeArgument(1)),
                        _ => return Err(mismatch(exp.span, &expected, &self.builtin("vector"))),
                    };
                    let length = self.refined(
                        "atom",
                        TypArgKind::NExp(Node::dummy(NExpKind::Constant(items.len().to_string()))),
                    );
                    self.scalar_equal(
                        &Scalar {
                            owner: expected,
                            index: 0,
                        },
                        &Scalar {
                            owner: length,
                            index: 0,
                        },
                        exp.span,
                    )?;
                    for item in items.iter().rev() {
                        tasks.push(Task {
                            exp: item,
                            expected: element.clone(),
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::Match { scrutinee, arms } => {
                    // OCaml check_exp E_match ~L2176 (check-only; infer stays unsupported).
                    // Scrutinee: infer normally; under `mapping_match` check <= option(Expected).
                    let mapping = exp.annotation.as_ref().is_some_and(|a| {
                        a.attributes.iter().any(|attr| attr.name == "mapping_match")
                    });
                    let scrut_ty = if mapping {
                        let mut interner = SimpleInterner::new();
                        let name = Name::dummy(interner.intern("option"));
                        let elem = interner.intern("__element");
                        let mut option_ty = Type::owned(
                            Node::dummy(TypKind::App {
                                name,
                                args: vec![Node::dummy(TypArgKind::Typ(Node::dummy(
                                    TypKind::Var(elem),
                                )))],
                            }),
                            Arc::new(interner),
                        );
                        option_ty.substitutions = Arc::new(Substitutions {
                            types: vec![(elem, expected.clone())],
                            scalars: Vec::new(),
                        });
                        tasks.push(Task {
                            exp: scrutinee,
                            expected: option_ty,
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                        self.infer(scrutinee, &scope)?
                    } else {
                        self.infer(scrutinee, &scope)?
                    };
                    // MatchCompleteness oracle (Lane-09 Pattern_completeness,
                    // Lane-26 crate::exp_match::MatchCompletenessOracle):
                    // exhaustiveness is not decided here; premises above are
                    // checked, constructor/list/vector patterns stay honest
                    // `Unsupported` via `self.pattern` below. Reference only.
                    fn _match_oracle_is_reference_only() {
                        fn assert_is_oracle<T: crate::exp_match::MatchCompletenessOracle>() {}
                    }
                    let _ = crate::exp_match::check_match
                        as fn(&[bool], bool) -> crate::exp_match::Plan;
                    let _ = crate::exp_match::SideCond::Completeness;
                    for arm in arms.iter().rev() {
                        let (pat, guard, body) = match &arm.kind {
                            PExpKind::PatExp { pat, body } => (pat, None, body),
                            PExpKind::PatWhen { pat, guard, body } => (pat, Some(guard), body),
                        };
                        let mut arm_scope = Scope {
                            locals: Vec::new(),
                            facts: Vec::new(),
                            parent: Some(scope.clone()),
                        };
                        self.pattern(pat, scrut_ty.clone(), false, &mut arm_scope)?;
                        let arm_scope = Arc::new(arm_scope);
                        if let Some(guard_exp) = guard {
                            let cond = self.condition(guard_exp, &arm_scope)?;
                            let body_scope = arm_scope.branch(cond.as_ref(), false);
                            tasks.push(Task {
                                exp: body,
                                expected: expected.clone(),
                                scope: body_scope,
                                returning: returning.clone(),
                            });
                            tasks.push(Task {
                                exp: guard_exp,
                                expected: self.builtin("bool"),
                                scope: arm_scope,
                                returning: returning.clone(),
                            });
                        } else {
                            tasks.push(Task {
                                exp: body,
                                expected: expected.clone(),
                                scope: arm_scope,
                                returning: returning.clone(),
                            });
                        }
                    }
                }
                ExpKind::Struct { name, fields } => {
                    // OCaml `check_exp` E_struct ~L2218: the expected type
                    // must be a declared record (`ExpectedMustBeRecord`), the
                    // struct name must match (`StructNameMustMatch`), every
                    // field is checked against its accessor payload and the
                    // literal owes full coverage (`FullFieldCoverage`).
                    let rectyp_id = self.struct_expected_record(&expected, exp.span)?;
                    match name {
                        StructName::Anon => {}
                        StructName::Id(id) => {
                            let found = self.name(id)?.to_owned();
                            if found != rectyp_id {
                                return Err(TypeError::mismatch(
                                    exp.span,
                                    rectyp_id.clone(),
                                    format!("struct {found}"),
                                ));
                            }
                        }
                    }
                    let record = self
                        .program
                        .record(&rectyp_id, self.position)
                        .expect("checked struct record");
                    {
                        let mut seen = std::collections::BTreeSet::new();
                        for fexp in fields.iter() {
                            let field_name = self.name(&fexp.field)?.to_owned();
                            if !record.fields.iter().any(|f| f.name == field_name) {
                                return Err(TypeError::unbound_var(fexp.field.span, field_name));
                            }
                            if !seen.insert(field_name.clone()) {
                                return Err(unsupported(
                                    fexp.field.span,
                                    format!("duplicate field '{field_name}'"),
                                ));
                            }
                        }
                        if seen.len() != record.fields.len() {
                            let missing: Vec<_> = record
                                .fields
                                .iter()
                                .map(|f| f.name.clone())
                                .filter(|n| !seen.contains(n))
                                .collect();
                            return Err(unsupported(
                                exp.span,
                                format!("struct literal missing fields: {}", missing.join(", ")),
                            ));
                        }
                    }
                    for fexp in fields.iter().rev() {
                        let field_name = self.name(&fexp.field)?.to_owned();
                        let field_typ = self.struct_field_type(
                            &rectyp_id,
                            &field_name,
                            &expected,
                            fexp.field.span,
                        )?;
                        tasks.push(Task {
                            exp: &fexp.value,
                            expected: field_typ,
                            scope: scope.clone(),
                            returning: returning.clone(),
                        });
                    }
                }
                ExpKind::VectorAppend(left, right) => {
                    let inferred_left = self.infer(left, &scope)?;
                    let inferred_right = self.infer(right, &scope)?;
                    let left_typ = self.resolve(&inferred_left)?;
                    let right_typ = self.resolve(&inferred_right)?;
                    let result = self.append_result(&left_typ, &right_typ, exp.span)?;
                    let result = self.value_type(exp, result);
                    self.compatible(&expected, &result, exp.span)?;
                }
                _ => {
                    return Err(unsupported(
                        exp.span,
                        "expression rule is not yet implemented in the canonical checker",
                    ))
                }
            }
        }
        Ok(())
    }
}
