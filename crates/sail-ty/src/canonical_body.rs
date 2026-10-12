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
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[derive(Clone, PartialEq, Eq)]
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
#[derive(Default)]
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
    proof_scope: Arc<Scope>,
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
        self.program.scattered_clause_count(ns.into(), name, self.position)
    }
    fn closed_at(&self, name: &str, position: usize) -> Option<crate::scattered_elab::Namespace> {
        // `scattered_closed_at` returns `program::ScatteredNamespace`; map
        // back variant-for-variant (same four variants, same order).
        self.program.scattered_closed_at(name, position).map(|ns| match ns {
            ScatteredNamespace::Function => crate::scattered_elab::Namespace::Function,
            ScatteredNamespace::Mapping => crate::scattered_elab::Namespace::Mapping,
            ScatteredNamespace::Variant => crate::scattered_elab::Namespace::Variant,
            ScatteredNamespace::Enum => crate::scattered_elab::Namespace::Enum,
        })
    }
}

pub(crate) fn check(
    program: &Program,
    def: &Def,
    it: Arc<SimpleInterner>,
    position: usize,
) -> TypeResult<usize> {
    let mut checker = Checker {
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
        proof_scope: Arc::default(),
    };
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
        DefKind::Scattered(scat) => {
            use crate::scattered_elab::{BodyPlan, ScatteredView};
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
            checker.measure(pat, exp, &args)?;
        }
        DefKind::LoopMeasures { name, measures } => {
            let count = crate::measure_check::check_loop_measures_target(
                program, name, &checker.it, measures, position, def.span,
            )?;
            debug_assert_eq!(count, measures.len());
            checker.loop_measures(measures)?;
        }
        DefKind::Impl(_)
        | DefKind::Mapdef { .. }
        | DefKind::Outcome { .. }
        | DefKind::Instantiation { .. }
        | DefKind::InternalMutrec(_) => {
            return Err(unsupported(
                def.span,
                "mapping, scattered, outcome or measure definition",
            ))
        }
        DefKind::Let { pat, exp } => {
            let scope = Arc::new(Scope::default());
            let typ = checker.infer(exp, &scope)?;
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
    Ok(checker.expressions)
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
    fn value_type(&mut self, exp: &Exp, declared: Type) -> Type {
        if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
            return typ.value.clone();
        }
        let shared: Arc<Type> = Arc::new(declared);
        let typ = self.open_value(&shared);
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
        let query =
            CanonicalQuery::equality(&self.proof_assumptions(&scope), left.view(), right.view())
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
        self.program
            .check_annotation(typ, &self.it, self.position, self.context.as_deref())?;
        let mut typ = Type::owned(typ.clone(), self.it.clone());
        if let Some(scope) = &self.context {
            typ.scope = scope.clone();
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
                if let Some(candidates) =
                    self.program.overload_candidates(res.overload, self.position)?
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
            let attempt = (|| {
                let TypKind::Fn { args, .. } = &callee.node().kind else {
                    return Err(unsupported(span,"overload candidate is not an elaborated function (mapping expansion may be required)"));
                };
                if args.len() != actual.len() {
                    return Err(arity(span, args.len(), actual.len()));
                }
                let callee = if callee.root.typschm.quant.is_empty() {
                    callee
                } else {
                    self.instantiate(callee, actual, expected, span)?
                };
                for (index, typ) in actual.iter().enumerate() {
                    self.compatible(&callee.child(Step::Argument(index)), typ, span)?;
                }
                if let Some(expected) = expected {
                    self.compatible(expected, &callee.child(Step::Result), span)?;
                }
                Ok(callee)
            })();
            match attempt {
                Ok(callee) => return Ok(callee),
                Err(error) if matches!(error.kind, TypeErrorKind::Unsupported(_)) => {
                    return Err(error)
                }
                Err(error) => last = Some(error),
            }
        }
        if single {
            return Err(last.expect("one attempted candidate"));
        }
        Err(TypeError {
            span,
            kind: TypeErrorKind::QuantError(format!(
                "no matching overload{}",
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
    /// Prove a vector-LHS bounds obligation through the canonical SMT backend.
    /// Mirrors `scalar_equal`: only a valid `program.prove` verdict is
    /// accepted, anything else is `ConstraintNotProved` (`Unknown` never
    /// becomes a proof). Emitter rejections (symbolic powers, numeric
    /// applications, unexpandable synonyms) surface as `Unsupported`.
    /// A syntactically decided obligation short-circuits without a query.
    fn prove_vector_bounds(
        &self,
        obligation: &crate::nexp::NConstraint,
        span: Span,
        what: &str,
    ) -> TypeResult<()> {
        if let Some(true) = obligation.simplify().as_bool() {
            return Ok(());
        }
        let mut it = SimpleInterner::new();
        let constraint = crate::conv::constraint_to_ast(obligation, &mut it);
        let scope = self.context.as_ref().map(|s| Type::root(s.clone()));
        let assumptions = self.proof_assumptions(&scope);
        let query = CanonicalQuery::implication(
            &assumptions,
            View {
                constraint: &constraint,
                context: Context {
                    interner: &it,
                    variables: &EMPTY_PROOF_VARS,
                    bindings: None,
                },
                negated: false,
            },
            None,
        )
        .map_err(|e| unsupported(span, e))?;
        if !self.program.prove(query).is_valid() {
            return Err(TypeError::constraint_not_proved(
                span,
                format!("{what}: {obligation}"),
            ));
        }
        Ok(())
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
        use crate::lexp_bounds::{
            check_index_typ, check_slice_root_len, element_typ, index_obligation, VectorRoot,
        };
        use crate::vector_lexp::{as_vector, as_vector_range};
        use crate::vector_lexp_bridge as bridge;
        if let Some((base, index)) = as_vector(lexp) {
            let base_typ = self.assignment_type(base, scope)?;
            let root = self.resolve(&base_typ)?;
            let vector_root =
                bridge::vector_root_of_canonical(root.node(), &*root.root.interner, lexp.span)?;
            let inferred = self.infer(index, scope)?;
            let index_typ = self.resolve(&inferred)?;
            let mini_index_typ = bridge::mini_typ_of_canonical(
                index_typ.node(),
                &*index_typ.root.interner,
                index.span,
            )?;
            check_index_typ(&mini_index_typ, index.span)?;
            let index_nexp = bridge::index_nexp_from_exp(index, &*self.it)?;
            let len = match &vector_root {
                VectorRoot::Vector { len, .. } | VectorRoot::Bitvector { len } => len.clone(),
            };
            self.prove_vector_bounds(
                &index_obligation(&index_nexp, &len),
                lexp.span,
                "vector index bounds",
            )?;
            let mut it = SimpleInterner::new();
            let lifted = bridge::ast_of_mini_typ(&element_typ(&vector_root), &mut it);
            return Ok(Type::owned(lifted, Arc::new(it)));
        }
        if let Some((base, first, second)) = as_vector_range(lexp) {
            let base_typ = self.assignment_type(base, scope)?;
            let root = self.resolve(&base_typ)?;
            let vector_root =
                bridge::vector_root_of_canonical(root.node(), &*root.root.interner, lexp.span)?;
            // Ein `vector`-Root ist hier ein Mismatch (OCaml:
            // "Cannot assign slice of non vector type"); erst danach
            // käme die ordnungsabhängige Obligation.
            let _len = check_slice_root_len(&vector_root, lexp.span)?;
            for bound in [first, second] {
                let inferred = self.infer(bound, scope)?;
                let bound_typ = self.resolve(&inferred)?;
                let mini = bridge::mini_typ_of_canonical(
                    bound_typ.node(),
                    &*bound_typ.root.interner,
                    bound.span,
                )?;
                check_index_typ(&mini, bound.span)?;
                let _ = bridge::index_nexp_from_exp(bound, &*self.it)?;
            }
            return Err(unsupported(
                lexp.span,
                "vector slice assignment needs a default order (Env.get_default_order); no implicit fallback",
            ));
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
                            results.push(declared);
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
        Ok(instantiated.child(Step::Result))
    }

    fn compatible(&self, expected: &Type, found: &Type, span: Span) -> TypeResult<()> {
        let mut pairs = vec![(expected.clone(), found.clone())];
        while let Some((expected, found)) = pairs.pop() {
            let expected = self.resolve(&expected)?;
            let found = self.resolve(&found)?;
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
                PatKind::Lit(lit) => {
                    let actual = self.literal(lit)?;
                    self.compatible(&typ, &actual, pat.span)?;
                }
                _ => {
                    return Err(unsupported(
                        pat.span,
                        "constructor, vector, list, disjunctive or dependent pattern binding",
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
        let typ = Type::root(scheme);
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
        let mut scope = Scope::default();
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
                &name,
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
        }
        let mut work = vec![Synthesis::Enter(exp)];
        let mut results = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Synthesis::Enter(exp) => {
                    if let Some(typ) = self.inferred.get(&(exp as *const Exp)) {
                        results.push(typ.value.clone());
                        continue;
                    }
                    let declared = match &exp.kind {
                        ExpKind::Lit(lit) => self.literal(lit)?,
                        ExpKind::Id(name) => self.id(name, scope)?,
                        ExpKind::Typ { typ, .. } => self.annotation(typ)?,
                        ExpKind::Tuple(items) => {
                            work.push(Synthesis::Tuple(exp, items.len()));
                            work.extend(items.iter().rev().map(Synthesis::Enter));
                            continue;
                        }
                        ExpKind::Vector(items) => {
                            let Some(first) = items.first() else {
                                return Err(unsupported(
                                    exp.span,
                                    "cannot infer the element type of an empty vector",
                                ));
                            };
                            work.push(Synthesis::Vector(exp, items.len()));
                            work.push(Synthesis::Enter(first));
                            continue;
                        }
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
                            work.push(Synthesis::Struct(exp, fields.len()));
                            work.extend(fields.iter().rev().map(|f| Synthesis::Enter(&f.value)));
                            continue;
                        }
                        // LANE-45 infer `E_struct_update` ~L3823: infer the
                        // base; fields are checked in
                        // `Synthesis::StructUpdate`.
                        ExpKind::StructUpdate { record, fields } => {
                            work.push(Synthesis::StructUpdate(exp, fields.len()));
                            work.push(Synthesis::Enter(record.as_ref()));
                            continue;
                        }
                        ExpKind::App { func, args } => {
                            let mut candidates = self.call_candidates(func, scope, args.len())?;
                            if candidates.len() != 1 {
                                work.push(Synthesis::Call(exp, candidates, args.len()));
                                work.extend(args.iter().rev().map(Synthesis::Enter));
                                continue;
                            }
                            let callee = candidates.pop().unwrap();
                            let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                                return Err(unsupported(exp.span, "application of non-function"));
                            };
                            if args.len() != formal.len() {
                                return Err(arity(exp.span, formal.len(), args.len()));
                            }
                            if callee.root.typschm.quant.is_empty() {
                                self.resolve(&callee.child(Step::Result))?
                            } else {
                                work.push(Synthesis::Call(exp, vec![callee], args.len()));
                                work.extend(args.iter().rev().map(Synthesis::Enter));
                                continue;
                            }
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
                Synthesis::Call(exp, candidates, count) => {
                    let actual = results.split_off(results.len() - count);
                    let callee = self.select_call(candidates, &actual, None, exp.span)?;
                    let result = self.resolve(&callee.child(Step::Result))?;
                    // value_type inserts the shared declared/value pair; no
                    // second insert here (that would duplicate the entry).
                    let result = self.value_type(exp, result);
                    self.calls.insert(exp as *const Exp, callee);
                    results.push(result);
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
                                        return Err(arity(fexp.value.span, items.len(), found.len()));
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
                                        return Err(arity(fexp.value.span, args.len(), found.len()));
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
                                                TypArgKind::NExp(_)
                                                | TypArgKind::Bool(_),
                                                TypArgKind::NExp(_)
                                                | TypArgKind::Bool(_),
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
                                                if let Some((symbol, _)) = variable.filter(
                                                    |(s, k)| {
                                                        quantified_scalars.iter().any(
                                                            |(bound, bound_kind)| {
                                                                bound == s && bound_kind == k
                                                            },
                                                        )
                                                    },
                                                ) {
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
                                args.push(Node::dummy(TypArgKind::Typ(Node::dummy(
                                    TypKind::Var(binder.kid),
                                ))));
                            }
                            Kind::Int => {
                                args.push(Node::dummy(TypArgKind::NExp(Node::dummy(
                                    NExpKind::Var(binder.kid),
                                ))));
                            }
                            Kind::Bool => {
                                args.push(Node::dummy(TypArgKind::Bool(Box::new(
                                    Node::dummy(ConstraintKind::Var(binder.kid)),
                                ))));
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
                        let inferred = self.infer(&fexp.value, scope)?;
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
        mut callee: Type,
        actual: &[Type],
        expected: Option<&Type>,
        span: Span,
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
        let mut bindings: Vec<(Symbol, Type)> = Vec::new();
        let mut scalar_bindings: Vec<(Symbol, Scalar)> = Vec::new();
        let mut pairs: Vec<_> = actual
            .iter()
            .enumerate()
            .map(|(i, typ)| (callee.child(Step::Argument(i)), typ.clone(), false))
            .collect();
        if let Some(expected) = expected {
            pairs.push((callee.child(Step::Result), expected.clone(), true));
        }
        while let Some((formal, actual, result_context)) = pairs.pop() {
            let formal = self.resolve(&formal)?;
            let actual = self.resolve(&actual)?;
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
                        return Err(unsupported(span, "synonym expansion during instantiation"));
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
        let assumptions = self.proof_assumptions(&scope);
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
            let expected = self.resolve(&expected)?;
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
                    for (index, item) in items.iter().enumerate().rev() {
                        let expected = if index + 1 == items.len() {
                            expected.clone()
                        } else {
                            self.builtin("unit")
                        };
                        tasks.push(Task {
                            exp: item,
                            expected,
                            scope: scope.clone(),
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
                    let mut candidates = self.call_candidates(func, &scope, args.len())?;
                    let callee = if let Some(callee) = self.calls.get(&(exp as *const Exp)) {
                        callee.clone()
                    } else if candidates.len() != 1 || !candidates[0].root.typschm.quant.is_empty()
                    {
                        let actual = args
                            .iter()
                            .map(|arg| self.infer(arg, &scope))
                            .collect::<TypeResult<Vec<_>>>()?;
                        self.select_call(candidates, &actual, Some(&expected), exp.span)?
                    } else {
                        candidates.pop().unwrap()
                    };
                    let TypKind::Fn { args: formal, .. } = &callee.node().kind else {
                        return Err(unsupported(exp.span, "application of non-function"));
                    };
                    if args.len() != formal.len() {
                        return Err(arity(exp.span, formal.len(), args.len()));
                    }
                    let result = self.resolve(&callee.child(Step::Result))?;
                    let result = self.value_type(exp, result);
                    self.compatible(&expected, &result, exp.span)?;
                    for (index, arg) in args.iter().enumerate().rev() {
                        tasks.push(Task {
                            exp: arg,
                            expected: callee.child(Step::Argument(index)),
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
                        expected.name().is_some_and(|name| {
                            self.program.record(name, self.position).is_some()
                        })
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
                        let field_typ = self.struct_field_type(
                            &record_name,
                            field_name,
                            &expected,
                            fexp.span,
                        )?;
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
                ExpKind::Let { pat, bind, body } => {
                    let bound = self.infer(bind, &scope)?;
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
                            PExpKind::PatWhen { pat, guard, body } => {
                                (pat, Some(guard), body)
                            }
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
                ExpKind::Return(child) => tasks.push(Task {
                    exp: child,
                    expected: returning.clone(),
                    scope,
                    returning,
                }),
                ExpKind::Undef => {}
                ExpKind::Sizeof(_) => {
                    return Err(unsupported(exp.span, "dependent size expression typing"))
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
                            PExpKind::PatWhen { pat, guard, body } => {
                                (pat, Some(guard), body)
                            }
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
