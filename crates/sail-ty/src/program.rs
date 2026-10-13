#![forbid(unsafe_code)]
//! Canonical AST typing environment. Declaration histories provide immutable
//! source-position views without copying the whole environment per worker.
//! Function bodies stay in the driver's out-of-core store.

use crate::error::{TypeError, TypeErrorKind, TypeResult};
use sail_ast::*;
use sail_solver::{
    canonical::{CanonicalQuery, Context, Substitution, View},
    SmtLibSolver, Solver, SolverPool, SolverQuery,
};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Scheme {
    pub typschm: TypSchm,
    pub interner: Arc<SimpleInterner>,
}
impl Drop for Scheme {
    fn drop(&mut self) {
        let quant = std::mem::take(&mut self.typschm.quant);
        let typ = std::mem::replace(&mut self.typschm.typ, Node::dummy(TypKind::Unknown));
        dispose_signature(quant, typ);
    }
}
fn dispose_signature(quant: Vec<QuantItem>, typ: Typ) {
    sail_ast::storage::drop_definition(Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: Name::dummy(Symbol::DUMMY),
        typschm: TypSchm { quant, typ },
        extern_opt: None,
    })));
}
#[derive(Clone, Debug)]
struct Binding<T> {
    position: usize,
    value: T,
}
#[derive(Clone, Debug)]
struct Constructor {
    args: Vec<Kind>,
    result: Kind,
    proof: Option<Arc<ConstructorProof>>,
    synonym: Option<Arc<Scheme>>,
}
#[derive(Debug)]
struct ConstructorProof {
    quant: Vec<QuantItem>,
    interner: Arc<SimpleInterner>,
}
#[derive(Clone, Debug)]
struct Value {
    scheme: Arc<Scheme>,
    enumeration_member: bool,
    inline: bool,
}

/// One preserved record field payload: the field type as declared,
/// with its source span. Names are resolved strings so lookups never
/// depend on a definition-local interner.
#[derive(Clone, Debug)]
pub struct RecordField {
    pub name: String,
    pub typ: Typ,
    pub span: Span,
}

/// Preserved `struct` payload (OCaml `Env.records`): the quantified
/// telescope plus every field type. The constructor-kind signature
/// lives in [`Program::constructors`]; this table keeps the field
/// payloads that well-formedness checking would otherwise drop.
#[derive(Clone, Debug)]
pub struct RecordDef {
    pub quant: Vec<QuantItem>,
    pub interner: Arc<SimpleInterner>,
    pub fields: Vec<RecordField>,
}
impl Drop for RecordDef {
    fn drop(&mut self) {
        let quant = std::mem::take(&mut self.quant);
        let fields = std::mem::take(&mut self.fields);
        let ast_fields = fields
            .into_iter()
            .map(|field| (Name::dummy(Symbol::DUMMY), field.typ))
            .collect();
        sail_ast::storage::drop_definition(Node::dummy(DefKind::Type(Node::dummy(
            TypeDefKind::Record {
                name: Name::dummy(Symbol::DUMMY),
                quant,
                fields: ast_fields,
                wf: false,
            },
        ))));
    }
}

/// One preserved union constructor payload: the constructor argument
/// type as declared, with its source span.
#[derive(Clone, Debug)]
pub struct UnionCtor {
    pub name: String,
    pub typ: Typ,
    pub span: Span,
}

/// Preserved `union` payload (OCaml `Env.unions`): the quantified
/// telescope plus every constructor argument type. Each constructor
/// additionally keeps its `Fn(payload) -> union` scheme in
/// [`Program::values`]; this table keeps the grouped payload list.
#[derive(Clone, Debug)]
pub struct UnionDef {
    pub quant: Vec<QuantItem>,
    pub interner: Arc<SimpleInterner>,
    pub ctors: Vec<UnionCtor>,
}
impl Drop for UnionDef {
    fn drop(&mut self) {
        let quant = std::mem::take(&mut self.quant);
        let ctors = std::mem::take(&mut self.ctors);
        let ast_ctors = ctors
            .into_iter()
            .map(|ctor| TypeUnion {
                id: NodeId::DUMMY,
                span: ctor.span,
                typ: ctor.typ,
                ctor: Name::dummy(Symbol::DUMMY),
            })
            .collect();
        sail_ast::storage::drop_definition(Node::dummy(DefKind::Type(Node::dummy(
            TypeDefKind::Variant {
                name: Name::dummy(Symbol::DUMMY),
                quant,
                ctors: ast_ctors,
                wf: false,
            },
        ))));
    }
}

/// Preserved mapping header (OCaml `Env` mapping entry): the quantified
/// telescope plus the bidirectional type. Clause bodies stay in the
/// driver's out-of-core store; only the header lives here so workers can
/// expand mapping applications (`<m>_forwards` / `<m>_backwards`,
/// `type_check.ml` mapping arms) without seeing a body.
#[derive(Clone, Debug)]
pub struct MappingDef {
    pub quant: Vec<QuantItem>,
    pub typ: Typ,
    pub interner: Arc<SimpleInterner>,
    pub span: Span,
}
impl Drop for MappingDef {
    fn drop(&mut self) {
        let quant = std::mem::take(&mut self.quant);
        let typ = std::mem::replace(&mut self.typ, Node::dummy(TypKind::Unknown));
        dispose_signature(quant, typ);
    }
}

/// Namespace of a `scattered` block. Functions, mappings, unions and enums
/// live in separate namespaces (OCaml `parse_ast` `scattered_def_aux`);
/// a `function f` and a `mapping f` may coexist, so the table key always
/// carries the namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScatteredNamespace {
    Function,
    Mapping,
    Variant,
    Enum,
}

impl ScatteredNamespace {
    /// Deterministic key prefix for reports and clause summaries.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Mapping => "mapping",
            Self::Variant => "union",
            Self::Enum => "enum",
        }
    }
}

impl From<crate::scattered_elab::Namespace> for ScatteredNamespace {
    fn from(ns: crate::scattered_elab::Namespace) -> Self {
        match ns {
            crate::scattered_elab::Namespace::Function => Self::Function,
            crate::scattered_elab::Namespace::Mapping => Self::Mapping,
            crate::scattered_elab::Namespace::Variant => Self::Variant,
            crate::scattered_elab::Namespace::Enum => Self::Enum,
        }
    }
}

/// One preserved `scattered` opening (OCaml `SD_function` / `SD_mapping` /
/// `SD_variant` / `SD_enum`): the optional header annotation plus whether
/// the matching `end` was declared. Clause bodies (`SD_funcl` /
/// `SD_mapcl` / `SD_unioncl` / `SD_enumcl`) are counted in
/// [`Program::scattered_clauses`]; bodies stay in the driver store.
#[derive(Clone, Debug)]
pub struct ScatteredDecl {
    pub namespace: ScatteredNamespace,
    pub name: String,
    pub span: Span,
    pub quant: Vec<QuantItem>,
    pub ret: Option<Typ>,
    pub interner: Option<Arc<SimpleInterner>>,
    pub closed: bool,
}
impl Drop for ScatteredDecl {
    fn drop(&mut self) {
        let quant = std::mem::take(&mut self.quant);
        let typ = self.ret.take().unwrap_or_else(|| Node::dummy(TypKind::Unknown));
        dispose_signature(quant, typ);
    }
}

/// Preserved outcome header (OCaml `Env.add_outcome`): the outcome
/// parameters plus the outcome type scheme. The per-outcome local
/// environment (inner `val` schemes) lives in
/// [`Program::outcome_vals`]; inner bodies stay in the driver store.
#[derive(Clone, Debug)]
pub struct OutcomeDef {
    pub params: Vec<QuantItem>,
    pub typschm: TypSchm,
    pub interner: Arc<SimpleInterner>,
    pub span: Span,
}
impl Drop for OutcomeDef {
    fn drop(&mut self) {
        let params = std::mem::take(&mut self.params);
        let typschm = std::mem::replace(
            &mut self.typschm,
            TypSchm {
                quant: Vec::new(),
                typ: Node::dummy(TypKind::Unknown),
            },
        );
        dispose_signature(params, Node::dummy(TypKind::Unknown));
        dispose_signature(typschm.quant, typschm.typ);
    }
}

/// Canonical declaration tables, frozen before parallel body checking. Only
/// signatures/type definitions belong here, never function bodies.
///
/// `records` / `unions` mirror OCaml `Env.records` / `Env.unions`:
/// the constructor-kind signature lives in `constructors`, while these
/// tables preserve the field/constructor *payloads* (each field type as
/// declared) alongside the quantified telescope. `accessors` mirrors
/// OCaml `Env.accessors`: one `Fn(record) -> field` scheme per
/// `(record, field)` pair. All three follow the same declaration
/// history (`Binding` + source `position`): a worker at `position`
/// never sees a later declaration.
pub struct Program {
    constructors: BTreeMap<String, Vec<Binding<Constructor>>>,
    values: BTreeMap<String, Vec<Binding<Value>>>,
    overloads: BTreeMap<String, Vec<Binding<Vec<String>>>>,
    registers: BTreeMap<String, Vec<Binding<Arc<Scheme>>>>,
    records: BTreeMap<String, Vec<Binding<RecordDef>>>,
    unions: BTreeMap<String, Vec<Binding<UnionDef>>>,
    accessors: BTreeMap<(String, String), Vec<Binding<Arc<Scheme>>>>,
    mappings: BTreeMap<String, Vec<Binding<MappingDef>>>,
    scattered: BTreeMap<(ScatteredNamespace, String), Vec<Binding<ScatteredDecl>>>,
    scattered_clauses: BTreeMap<(ScatteredNamespace, String), Vec<Binding<usize>>>,
    outcomes: BTreeMap<String, Vec<Binding<OutcomeDef>>>,
    outcome_vals: BTreeMap<(String, String), Vec<Binding<Arc<Scheme>>>>,
    impls: Vec<Binding<String>>,
    measures: BTreeMap<String, Vec<Binding<Span>>>,
    loop_measures: BTreeMap<String, Vec<Binding<Span>>>,
    instantiations: Vec<Binding<(String, Span)>>,
    constraints: Vec<Binding<(Arc<Constraint>, Arc<SimpleInterner>)>>,
    solvers: SolverPool<SmtLibSolver>,
    default_order: Option<(Order, usize)>,
}
impl Default for Program {
    fn default() -> Self {
        Self {
            constructors: BTreeMap::new(),
            values: BTreeMap::new(),
            overloads: BTreeMap::new(),
            registers: BTreeMap::new(),
            records: BTreeMap::new(),
            unions: BTreeMap::new(),
            accessors: BTreeMap::new(),
            mappings: BTreeMap::new(),
            scattered: BTreeMap::new(),
            scattered_clauses: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            outcome_vals: BTreeMap::new(),
            impls: Vec::new(),
            measures: BTreeMap::new(),
            loop_measures: BTreeMap::new(),
            instantiations: Vec::new(),
            constraints: Vec::new(),
            solvers: SolverPool::with_default_cache(SmtLibSolver::new),
            default_order: None,
        }
    }
}

fn at<T>(history: &[Binding<T>], position: usize) -> Option<&T> {
    let index = history.partition_point(|binding| binding.position <= position);
    index.checked_sub(1).map(|index| &history[index].value)
}
fn text(it: &impl Interner, symbol: Symbol, span: Span) -> TypeResult<&str> {
    it.resolve(symbol)
        .ok_or_else(|| TypeError::unbound_var(span, format!("symbol {symbol}")))
}
fn failure(span: Span, message: impl Into<String>) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::QuantError(message.into()),
    }
}
fn constructor(args: &[Kind], result: Kind) -> Constructor {
    Constructor {
        args: args.to_vec(),
        result,
        proof: None,
        synonym: None,
    }
}
fn builtin(name: &str) -> Option<Constructor> {
    use Kind::{Bool, Int, Type};
    Some(match name {
        "range" => constructor(&[Int, Int], Type),
        "atom" | "implicit" | "bitvector" | "itself" => constructor(&[Int], Type),
        "vector" => constructor(&[Int, Type], Type),
        "register" | "list" => constructor(&[Type], Type),
        "atom_bool" => constructor(&[Bool], Type),
        "unit"
        | "int"
        | "nat"
        | "bool"
        | "real"
        | "string"
        | "string_literal"
        | "float16"
        | "float32"
        | "float64"
        | "float128"
        | "float_rounding_mode" => constructor(&[], Type),
        "abs" => constructor(&[Int], Int),
        "mod" | "div" => constructor(&[Int, Int], Int),
        // Core NC_app("not", [A_bool _]) is emitted by initial_check rather
        // than declared in Sail. See type_check.ml's rewrite_nc rule.
        "not" => constructor(&[Bool], Bool),
        _ => return None,
    })
}
fn quant_args(quant: &[QuantItem]) -> Vec<Kind> {
    quant
        .iter()
        .filter_map(|q| match q {
            QuantItem::Id(k) => Some(k.kind),
            QuantItem::Constraint(_) => None,
        })
        .collect()
}

impl Program {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn prove(&self, query: CanonicalQuery) -> sail_solver::SolverVerdict {
        let query = SolverQuery::new(sail_solver::Constraint::SmtLib(query)).with_timeout(3000);
        let cache = self.solvers.cache();
        cache.lookup(&query).unwrap_or_else(|| {
            let verdict = self.solvers.checkout().prove(&query);
            cache.store(&query, verdict);
            verdict
        })
    }
    pub(crate) fn constraints(
        &self,
        position: usize,
    ) -> impl Iterator<Item = (&Constraint, &SimpleInterner)> {
        self.constraints
            .iter()
            .filter(move |b| b.position <= position)
            .map(|b| (b.value.0.as_ref(), b.value.1.as_ref()))
    }

    /// Check one loaded definition against its immutable source-position view.
    /// This method owns no function body and can run on bounded worker pools.
    pub fn check_definition(
        &self,
        def: &Def,
        it: Arc<SimpleInterner>,
        position: usize,
    ) -> TypeResult<usize> {
        crate::canonical_body::check(self, def, it, position)
    }

    /// Check and retain a declaration in source order. Repeated val specs
    /// create history entries: an earlier body keeps its earlier signature.
    pub fn declare(
        &mut self,
        def: &Def,
        it: Arc<SimpleInterner>,
        position: usize,
    ) -> TypeResult<()> {
        match &def.kind {
            DefKind::Type(typ) => {
                let (name, quant, result) = match &typ.kind {
                    TypeDefKind::Abbrev { name, quant, arg } => {
                        (name, quant.as_slice(), arg_kind(arg))
                    }
                    TypeDefKind::Record { name, quant, .. }
                    | TypeDefKind::Variant { name, quant, .. } => {
                        (name, quant.as_slice(), Kind::Type)
                    }
                    TypeDefKind::Enum { name, .. } | TypeDefKind::Bitfield { name, .. } => {
                        (name, &[][..], Kind::Type)
                    }
                    TypeDefKind::Abstract { name, kind } => (name, &[][..], *kind),
                };
                let name = text(it.as_ref(), name.symbol, typ.span)?.to_owned();
                if builtin(&name).is_some() || self.constructors.contains_key(&name) {
                    return Err(failure(
                        typ.span,
                        format!("type constructor '{name}' is already defined"),
                    ));
                }
                let signature = Constructor {
                    args: quant_args(quant),
                    result,
                    proof: quant.iter().any(|item| matches!(item, QuantItem::Constraint(c) if !matches!(c.kind, ConstraintKind::True))).then(|| Arc::new(ConstructorProof {quant:quant.to_vec(),interner:it.clone()})),
                    synonym:match &typ.kind {
                        TypeDefKind::Abbrev {arg:Node {kind:TypArgKind::Typ(typ),..},..}=>Some(Arc::new(Scheme {typschm:TypSchm {quant:quant.to_vec(),typ:typ.clone()},interner:it.clone()})),
                        _=>None,
                    },
                };
                // OCaml's forbid_recursive_types checks record/union payloads
                // before inserting their constructor into the environment.
                let mut checker = WellFormed::new(self, &it, position);
                checker.quant(quant)?;
                match &typ.kind {
                    TypeDefKind::Abbrev { arg, .. } => checker.run(vec![Task::Arg(arg, result)])?,
                    TypeDefKind::Record { fields, .. } => {
                        checker.run(fields.iter().map(|(_, typ)| Task::Typ(typ)).collect())?
                    }
                    TypeDefKind::Variant { ctors, .. } => {
                        checker.run(ctors.iter().map(|ctor| Task::Typ(&ctor.typ)).collect())?
                    }
                    TypeDefKind::Bitfield { typ, fields, .. } => {
                        let mut tasks = vec![Task::Typ(typ)];
                        tasks.extend(fields.iter().map(|(_, range)| Task::Range(range)));
                        checker.run(tasks)?;
                    }
                    TypeDefKind::Enum { .. } | TypeDefKind::Abstract { .. } => {}
                }
                let mut members = Vec::new();
                match &typ.kind {
                    TypeDefKind::Enum {
                        name,
                        members: names,
                        ..
                    } => {
                        for member in names {
                            members.push((
                                text(it.as_ref(), member.symbol, member.span)?.to_owned(),
                                member.span,
                                Value {
                                    inline: false,
                                    scheme: Arc::new(Scheme {
                                        typschm: TypSchm {
                                            quant: vec![],
                                            typ: Node::dummy(TypKind::Id(name.clone())),
                                        },
                                        interner: it.clone(),
                                    }),
                                    enumeration_member: true,
                                },
                            ));
                        }
                    }
                    TypeDefKind::Variant {
                        name, quant, ctors, ..
                    } => {
                        let args: Vec<_> = quant
                            .iter()
                            .filter_map(|item| match item {
                                QuantItem::Constraint(_) => None,
                                QuantItem::Id(binder) => Some(Node::dummy(match binder.kind {
                                    Kind::Type => {
                                        TypArgKind::Typ(Node::dummy(TypKind::Var(binder.kid)))
                                    }
                                    Kind::Int => {
                                        TypArgKind::NExp(Node::dummy(NExpKind::Var(binder.kid)))
                                    }
                                    Kind::Bool => TypArgKind::Bool(Box::new(Node::dummy(
                                        ConstraintKind::Var(binder.kid),
                                    ))),
                                })),
                            })
                            .collect();
                        let ret = Node::dummy(if args.is_empty() {
                            TypKind::Id(name.clone())
                        } else {
                            TypKind::App {
                                name: name.clone(),
                                args,
                            }
                        });
                        for ctor in ctors {
                            let scheme = Scheme {
                                typschm: TypSchm {
                                    quant: quant.clone(),
                                    typ: Node::dummy(TypKind::Fn {
                                        args: vec![ctor.typ.clone()],
                                        ret: Box::new(ret.clone()),
                                    }),
                                },
                                interner: it.clone(),
                            };
                            members.push((
                                text(it.as_ref(), ctor.ctor.symbol, ctor.span)?.to_owned(),
                                ctor.span,
                                Value {
                                    inline: false,
                                    scheme: Arc::new(scheme),
                                    enumeration_member: false,
                                },
                            ));
                        }
                    }
                    _ => {}
                }
                let mut seen = std::collections::BTreeSet::new();
                for (name, span, _) in &members {
                    if !seen.insert(name)
                        || self.values.contains_key(name)
                        || self.registers.contains_key(name)
                    {
                        return Err(failure(
                            *span,
                            format!("constructor/member '{name}' is already bound"),
                        ));
                    }
                }
                // Preserve record/union field payloads alongside the
                // constructor entry (OCaml `Env.records` / `Env.unions` /
                // `Env.accessors`). Histories share this declaration's
                // `position`, so a source-position view never sees a later
                // declaration. Duplicate fields within one struct are
                // rejected; duplicate union constructors are already
                // rejected by the member check above.
                let record_payload = match &typ.kind {
                    TypeDefKind::Record { quant, fields, .. } => {
                        let mut seen_fields = std::collections::BTreeSet::new();
                        let mut kept = Vec::with_capacity(fields.len());
                        for (field_name, field_typ) in fields {
                            let field =
                                text(it.as_ref(), field_name.symbol, field_name.span)?.to_owned();
                            if !seen_fields.insert(field.clone()) {
                                return Err(failure(
                                    field_name.span,
                                    format!("duplicate field '{field}' in struct '{name}'"),
                                ));
                            }
                            kept.push(RecordField {
                                name: field,
                                typ: field_typ.clone(),
                                span: field_name.span,
                            });
                        }
                        Some(RecordDef {
                            quant: quant.clone(),
                            interner: it.clone(),
                            fields: kept,
                        })
                    }
                    _ => None,
                };
                let union_payload = match &typ.kind {
                    TypeDefKind::Variant { quant, ctors, .. } => {
                        let mut kept = Vec::with_capacity(ctors.len());
                        for ctor in ctors {
                            kept.push(UnionCtor {
                                name: text(it.as_ref(), ctor.ctor.symbol, ctor.span)?.to_owned(),
                                typ: ctor.typ.clone(),
                                span: ctor.span,
                            });
                        }
                        Some(UnionDef {
                            quant: quant.clone(),
                            interner: it.clone(),
                            ctors: kept,
                        })
                    }
                    _ => None,
                };
                // Field accessors mirror OCaml `Env.accessors`: one
                // `Fn(record) -> field` scheme per `(record, field)` pair,
                // quantified over the record telescope.
                let accessor_payload = match (&typ.kind, &record_payload) {
                    (TypeDefKind::Record { name: rec_name, quant, .. }, Some(record)) => {
                        let args: Vec<_> = quant
                            .iter()
                            .filter_map(|item| match item {
                                QuantItem::Constraint(_) => None,
                                QuantItem::Id(binder) => Some(Node::dummy(match binder.kind {
                                    Kind::Type => {
                                        TypArgKind::Typ(Node::dummy(TypKind::Var(binder.kid)))
                                    }
                                    Kind::Int => {
                                        TypArgKind::NExp(Node::dummy(NExpKind::Var(binder.kid)))
                                    }
                                    Kind::Bool => TypArgKind::Bool(Box::new(Node::dummy(
                                        ConstraintKind::Var(binder.kid),
                                    ))),
                                })),
                            })
                            .collect();
                        let record_typ = Node::dummy(if args.is_empty() {
                            TypKind::Id(rec_name.clone())
                        } else {
                            TypKind::App {
                                name: rec_name.clone(),
                                args,
                            }
                        });
                        record
                            .fields
                            .iter()
                            .map(|field| {
                                (
                                    field.name.clone(),
                                    Arc::new(Scheme {
                                        typschm: TypSchm {
                                            quant: quant.clone(),
                                            typ: Node::dummy(TypKind::Fn {
                                                args: vec![record_typ.clone()],
                                                ret: Box::new(field.typ.clone()),
                                            }),
                                        },
                                        interner: it.clone(),
                                    }),
                                )
                            })
                            .collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                self.constructors.entry(name.clone()).or_default().push(Binding {
                    position,
                    value: signature,
                });
                for (name, _, value) in members {
                    self.values
                        .entry(name)
                        .or_default()
                        .push(Binding { position, value });
                }
                if let Some(record) = record_payload {
                    self.records.entry(name.clone()).or_default().push(Binding {
                        position,
                        value: record,
                    });
                }
                if let Some(union) = union_payload {
                    self.unions.entry(name.clone()).or_default().push(Binding {
                        position,
                        value: union,
                    });
                }
                for (field, scheme) in accessor_payload {
                    self.accessors
                        .entry((name.clone(), field))
                        .or_default()
                        .push(Binding {
                            position,
                            value: scheme,
                        });
                }
                Ok(())
            }
            DefKind::Val(val) => {
                self.check_scheme(&val.typschm, &it, position)?;
                if !matches!(
                    val.typschm.typ.kind,
                    TypKind::Fn { .. } | TypKind::Bidir { .. }
                ) {
                    return Err(TypeError {
                        span: val.span,
                        kind: TypeErrorKind::Unsupported("val signature synonym expansion".into()),
                    });
                }
                let name = text(it.as_ref(), val.name.symbol, val.span)?.to_owned();
                self.values.entry(name).or_default().push(Binding {
                    position,
                    value: Value {
                        inline: false,
                        enumeration_member: false,
                        scheme: Arc::new(Scheme {
                            typschm: val.typschm.clone(),
                            interner: it,
                        }),
                    },
                });
                Ok(())
            }
            DefKind::Fundef {
                annot: TAnnotOpt::Some { quant, typ: ret },
                funcls,
                ..
            } => {
                let name = function_name(funcls, &it, def.span)?.to_owned();
                if self.value(&name, position).is_some() {
                    return self.check_function_header(def, &it, position);
                }
                let [clause] = funcls.as_slice() else {
                    return Err(failure(
                        def.span,
                        "cannot infer a function signature from multiple clauses",
                    ));
                };
                let pat = match &clause.arm.kind {
                    PExpKind::PatExp { pat, .. } | PExpKind::PatWhen { pat, .. } => pat,
                };
                let mut interner = it;
                let arg = infer_pattern_type(pat, Arc::make_mut(&mut interner))?;
                let args = match arg.kind {
                    TypKind::Tuple(args) => args,
                    _ => vec![arg],
                };
                let scheme = Scheme {
                    typschm: TypSchm {
                        quant: quant.clone(),
                        typ: Node::dummy(TypKind::Fn {
                            args,
                            ret: Box::new(ret.clone()),
                        }),
                    },
                    interner,
                };
                self.check_scheme(&scheme.typschm, &scheme.interner, position)?;
                self.values.entry(name).or_default().push(Binding {
                    position,
                    value: Value {
                        scheme: Arc::new(scheme),
                        enumeration_member: false,
                        inline: true,
                    },
                });
                Ok(())
            }
            DefKind::Register(reg) => {
                self.check_scheme(
                    &TypSchm {
                        quant: vec![],
                        typ: reg.typ.clone(),
                    },
                    &it,
                    position,
                )?;
                let name = text(it.as_ref(), reg.name.symbol, reg.span)?.to_owned();
                self.registers.entry(name).or_default().push(Binding {
                    position,
                    value: Arc::new(Scheme {
                        typschm: TypSchm {
                            quant: vec![],
                            typ: reg.typ.clone(),
                        },
                        interner: it,
                    }),
                });
                Ok(())
            }
            DefKind::Constraint(constraint) => {
                WellFormed::new(self, &it, position).run(vec![Task::Constraint(constraint)])?;
                self.constraints.push(Binding {
                    position,
                    value: (Arc::new(constraint.clone()), it),
                });
                Ok(())
            }
            DefKind::Overload { name, overloads } => {
                let name = text(&*it, name.symbol, name.span)?.to_owned();
                if self.value(&name, position).is_some() || self.register(&name, position).is_some()
                {
                    return Err(failure(
                        def.span,
                        format!("overload '{name}' conflicts with an existing value"),
                    ));
                }
                let mut candidates = self.overload(&name, position).unwrap_or(&[]).to_vec();
                for candidate in overloads {
                    let candidate_name = text(&*it, candidate.symbol, candidate.span)?;
                    if self.value(candidate_name, position).is_none()
                        && self.register(candidate_name, position).is_none()
                        && self.overload(candidate_name, position).is_none()
                    {
                        return Err(TypeError::unbound_var(
                            candidate.span,
                            candidate_name.to_owned(),
                        ));
                    }
                    candidates.push(candidate_name.to_owned());
                }
                // OCaml extends the existing list at its end, preserving the
                // first matching candidate and source-position semantics.
                self.overloads.entry(name).or_default().push(Binding {
                    position,
                    value: candidates,
                });
                Ok(())
            }
            // Bodies and definitions requiring inference are checked by the
            // body pass; accepting their header does not accept the body.
            // Mapping, scattered and outcome *headers* are retained below
            // so later declarations and body workers see them at their
            // source position; clause bodies stay in the driver store.
            DefKind::Mapdef { name, annot, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                let TAnnotOpt::Some { quant, typ } = annot else {
                    return Err(TypeError {
                        span: def.span,
                        kind: TypeErrorKind::Unsupported(
                            "mapping signature synthesis from clauses requires pattern elaboration"
                                .into(),
                        ),
                    });
                };
                let mut checker = WellFormed::new(self, &it, position);
                checker.quant(quant)?;
                checker.run(vec![Task::Typ(typ)])?;
                if !matches!(typ.kind, TypKind::Bidir { .. }) {
                    return Err(failure(
                        typ.span,
                        format!("mapping '{name_text}' must have a bi-directional mapping type"),
                    ));
                }
                self.mappings.entry(name_text).or_default().push(Binding {
                    position,
                    value: MappingDef {
                        quant: quant.clone(),
                        typ: typ.clone(),
                        interner: it,
                        span: name.span,
                    },
                });
                Ok(())
            }
            DefKind::Scattered(scat) => self.declare_scattered(scat, &it, position, def.span),
            DefKind::Outcome {
                name,
                typschm,
                quant: params,
                defs,
            } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                if self.outcome(&name_text, position).is_some() {
                    return Err(failure(
                        name.span,
                        format!("outcome '{name_text}' is already defined"),
                    ));
                }
                // Header + innere Specs sehen die äußeren `with`-Binder
                // (B1: `val pa_bits` sieht `with 'pa`; OCaml `check_outcome`).
                self.check_outcome(typschm, params, defs, &it, position)?;
                self.outcomes.entry(name_text.clone()).or_default().push(Binding {
                    position,
                    value: OutcomeDef {
                        params: params.clone(),
                        typschm: typschm.clone(),
                        interner: it.clone(),
                        span: name.span,
                    },
                });
                // Outcome-local `val` schemes never leak into the global
                // table (OCaml checks inner defs in the outcome-local
                // environment built by `check_outcome`).
                for inner in defs {
                    if let DefKind::Val(val) = &inner.kind {
                        let val_name =
                            text(it.as_ref(), val.name.symbol, val.span)?.to_owned();
                        self.outcome_vals
                            .entry((name_text.clone(), val_name))
                            .or_default()
                            .push(Binding {
                                position,
                                value: Arc::new(Scheme {
                                    typschm: val.typschm.clone(),
                                    interner: it.clone(),
                                }),
                            });
                    }
                }
                Ok(())
            }
            DefKind::Impl(funcl) => {
                let name_text =
                    text(it.as_ref(), funcl.name.symbol, funcl.name.span)?.to_owned();
                self.impls.push(Binding {
                    position,
                    value: name_text,
                });
                Ok(())
            }
            DefKind::Measure { name, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                if self.value(&name_text, position).is_none() {
                    return Err(TypeError::unbound_var(name.span, name_text));
                }
                self.measures.entry(name_text).or_default().push(Binding {
                    position,
                    value: def.span,
                });
                Ok(())
            }
            DefKind::LoopMeasures { name, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                if self.value(&name_text, position).is_none() {
                    return Err(TypeError::unbound_var(name.span, name_text));
                }
                self.loop_measures.entry(name_text).or_default().push(Binding {
                    position,
                    value: def.span,
                });
                Ok(())
            }
            DefKind::Instantiation { name, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                if self.outcome(&name_text, position).is_none() {
                    return Err(failure(
                        name.span,
                        format!("instantiation of unknown outcome '{name_text}'"),
                    ));
                }
                self.instantiations.push(Binding {
                    position,
                    value: (name_text, def.span),
                });
                Ok(())
            }
            DefKind::Default(order) => self.set_default_order(*order, position, def.span),
            _ => Ok(()),
        }
    }

    /// Global default order (`default Order`, OCaml `Env.get_default_order`):
    /// set once, second set is an error; use before set behaves as unset.
    /// Position-checked: declarations before the order do not see it.
    fn set_default_order(&mut self, order: Order, position: usize, span: Span) -> TypeResult<()> {
        if self.default_order.is_some() {
            return Err(failure(
                span,
                "Cannot change default order once already set",
            ));
        }
        self.default_order = Some((order, position));
        Ok(())
    }

    /// Default order visible at `position`, else `None` (OCaml
    /// `get_default_order` errors; callers report honest `Unsupported`).
    #[must_use]
    pub fn default_order(&self, position: usize) -> Option<Order> {
        self.default_order
            .filter(|(_, set_at)| *set_at <= position)
            .map(|(order, _)| order)
    }

    /// Record one `scattered` definition header or clause. Openings push a
    /// [`ScatteredDecl`] history entry (with their header annotation when
    /// present); clauses bump the per-block clause counter; `end` closes
    /// the single open block of that name across namespaces, or fails
    /// when the target is missing or ambiguous.
    fn declare_scattered(
        &mut self,
        scat: &ScatteredDef,
        it: &Arc<SimpleInterner>,
        position: usize,
        _span: Span,
    ) -> TypeResult<()> {
        fn opening(
            program: &mut Program,
            namespace: ScatteredNamespace,
            name: &Name,
            annot: &TAnnotOpt,
            it: &Arc<SimpleInterner>,
            position: usize,
        ) -> TypeResult<()> {
            let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
            // Re-opening an unclosed block of the same namespace is a
            // duplicate, not a history entry (OCaml rejects it during
            // scattered elaboration).
            if program
                .scattered
                .get(&(namespace, name_text.clone()))
                .and_then(|history| at(history, position))
                .is_some_and(|decl| !decl.closed)
            {
                return Err(failure(
                    name.span,
                    format!(
                        "{} '{}' is already openly scattered",
                        namespace.prefix(),
                        name_text
                    ),
                ));
            }
            let (quant, ret, interner) = match annot {
                TAnnotOpt::None => (Vec::new(), None, None),
                TAnnotOpt::Some { quant, typ } => {
                    let mut checker = WellFormed::new(program, it, position);
                    checker.quant(quant)?;
                    checker.run(vec![Task::Typ(typ)])?;
                    (quant.clone(), Some(typ.clone()), Some(it.clone()))
                }
            };
            program
                .scattered
                .entry((namespace, name_text.clone()))
                .or_default()
                .push(Binding {
                    position,
                    value: ScatteredDecl {
                        namespace,
                        name: name_text,
                        span: name.span,
                        quant,
                        ret,
                        interner,
                        closed: false,
                    },
                });
            Ok(())
        }
        fn clause(
            program: &mut Program,
            namespace: ScatteredNamespace,
            name: &str,
            span: Span,
            position: usize,
        ) -> TypeResult<()> {
            let key = (namespace, name.to_owned());
            let open = program
                .scattered
                .get(&key)
                .and_then(|history| at(history, position))
                .is_some_and(|decl| !decl.closed);
            if !open {
                return Err(failure(
                    span,
                    format!(
                        "{} clause for '{}' has no open scattered block",
                        namespace.prefix(),
                        name
                    ),
                ));
            }
            let next = program
                .scattered_clauses
                .get(&key)
                .and_then(|history| at(history, position))
                .copied()
                .unwrap_or(0)
                + 1;
            program
                .scattered_clauses
                .entry(key)
                .or_default()
                .push(Binding {
                    position,
                    value: next,
                });
            Ok(())
        }
        match scat {
            ScatteredDef::Function { name, annot } => {
                opening(self, ScatteredNamespace::Function, name, annot, it, position)
            }
            ScatteredDef::Mapping { name, annot } => {
                opening(self, ScatteredNamespace::Mapping, name, annot, it, position)
            }
            ScatteredDef::Variant { name, quant } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                let mut checker = WellFormed::new(self, it, position);
                checker.quant(quant)?;
                self.scattered
                    .entry((ScatteredNamespace::Variant, name_text.clone()))
                    .or_default()
                    .push(Binding {
                        position,
                        value: ScatteredDecl {
                            namespace: ScatteredNamespace::Variant,
                            name: name_text,
                            span: name.span,
                            quant: quant.clone(),
                            ret: None,
                            interner: None,
                            closed: false,
                        },
                    });
                Ok(())
            }
            ScatteredDef::Unioncl { name, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                clause(self, ScatteredNamespace::Variant, &name_text, name.span, position)
            }
            ScatteredDef::Enum(name) => opening(
                self,
                ScatteredNamespace::Enum,
                name,
                &TAnnotOpt::None,
                it,
                position,
            ),
            ScatteredDef::Enumcl { enum_name, .. } => {
                let name_text =
                    text(it.as_ref(), enum_name.symbol, enum_name.span)?.to_owned();
                clause(self, ScatteredNamespace::Enum, &name_text, enum_name.span, position)
            }
            ScatteredDef::Funcl(funcl) => {
                let name_text =
                    text(it.as_ref(), funcl.name.symbol, funcl.name.span)?.to_owned();
                clause(
                    self,
                    ScatteredNamespace::Function,
                    &name_text,
                    funcl.name.span,
                    position,
                )
            }
            ScatteredDef::Mapcl { name, .. } => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                clause(self, ScatteredNamespace::Mapping, &name_text, name.span, position)
            }
            ScatteredDef::End(name) => {
                let name_text = text(it.as_ref(), name.symbol, name.span)?.to_owned();
                let mut open: Vec<(ScatteredNamespace, ScatteredDecl)> = Vec::new();
                for namespace in [
                    ScatteredNamespace::Function,
                    ScatteredNamespace::Mapping,
                    ScatteredNamespace::Variant,
                    ScatteredNamespace::Enum,
                ] {
                    if let Some(decl) = self
                        .scattered
                        .get(&(namespace, name_text.clone()))
                        .and_then(|history| at(history, position))
                        .filter(|decl| !decl.closed)
                    {
                        open.push((namespace, decl.clone()));
                    }
                }
                match open.len() {
                    1 => {
                        let (namespace, mut closed_decl) = open.pop().unwrap();
                        closed_decl.closed = true;
                        closed_decl.span = name.span;
                        self.scattered
                            .entry((namespace, name_text))
                            .or_default()
                            .push(Binding {
                                position,
                                value: closed_decl,
                            });
                        Ok(())
                    }
                    0 => Err(failure(
                        name.span,
                        format!("scattered end '{name_text}' has no open block"),
                    )),
                    _ => Err(TypeError {
                        span: name.span,
                        kind: TypeErrorKind::Unsupported(
                            "ambiguous scattered end across namespaces requires elaboration-order stack"
                                .into(),
                        ),
                    }),
                }
            }
        }
    }

    pub fn check_scheme(
        &self,
        scheme: &TypSchm,
        it: &SimpleInterner,
        position: usize,
    ) -> TypeResult<()> {
        self.check_scoped_scheme(scheme, it, position, &[])
    }

    /// Validate a type scheme with additional outer quantifiers in scope.
    /// Interners are definition-local, so outer binders match by name. Inner
    /// binders shadow outer ones: own quantifiers live in `vars`, outer
    /// binders in the `outer` fallback map (cf. `check_annotation`).
    pub(crate) fn check_scoped_scheme(
        &self,
        scheme: &TypSchm,
        it: &SimpleInterner,
        position: usize,
        outer: &[(&[QuantItem], &SimpleInterner)],
    ) -> TypeResult<()> {
        let mut checker = WellFormed::new(self, it, position);
        for (quant, scope_it) in outer {
            for item in *quant {
                if let QuantItem::Id(binder) = item {
                    let name = text(*scope_it, binder.kid, scheme.typ.span)?.to_owned();
                    checker.outer.insert(name.clone(), binder.kind);
                    checker.bind_proof_name(name);
                }
            }
        }
        for (quant, scope_it) in outer {
            for item in *quant {
                if let QuantItem::Constraint(constraint) = item {
                    checker.assumptions.push(Assumption {
                        constraint,
                        interner: *scope_it,
                        variables: checker.proof_vars.clone(),
                    });
                }
            }
        }
        checker.quant(&scheme.quant)?;
        checker.run(vec![Task::Typ(&scheme.typ)])
    }

    /// Validate an outcome header and its nested specs. The outer `with`
    /// binders (`params`) plus the outcome type-scheme quantifiers scope over
    /// every nested scheme (OCaml `check_outcome`: `add_typquant params`,
    /// then `check_defs` in the local environment, `type_check.ml:5379ff`).
    /// Only `val`/`impl` members are admitted, as in OCaml
    /// (`valid_outcome_def`). Implementation bodies additionally need the
    /// outcome environment in the body checker (OCaml `check_impldef`); the
    /// declaration pass only admits them, it does not check their bodies.
    pub(crate) fn check_outcome(
        &self,
        typschm: &TypSchm,
        params: &[QuantItem],
        defs: &[Def],
        it: &SimpleInterner,
        position: usize,
    ) -> TypeResult<()> {
        // The header sees the outer binders, mirroring `wf_typschm local_env`.
        self.check_scoped_scheme(typschm, it, position, &[(params, it)])?;
        for def in defs {
            match &def.kind {
                // Inner val specs see outer plus header quantifiers, mirroring
                // `check_defs local_env` (B1: `val pa_bits` sees `with 'pa`).
                // Like OCaml `check_val_spec`, only well-formedness is
                // required here, never an arrow shape.
                DefKind::Val(val) => self.check_scoped_scheme(
                    &val.typschm,
                    it,
                    position,
                    &[(params, it), (&typschm.quant, it)],
                )?,
                DefKind::Impl(_) => {}
                _ => {
                    return Err(failure(
                        def.span,
                        "forbidden definition in outcome block",
                    ))
                }
            }
        }
        Ok(())
    }

    /// Validate loaded clauses against their source-position signature. Inline
    /// annotations store a return type, never an already constructed arrow.
    pub(crate) fn check_function_header(
        &self,
        def: &Def,
        it: &SimpleInterner,
        position: usize,
    ) -> TypeResult<()> {
        let DefKind::Fundef { annot, funcls, .. } = &def.kind else {
            return Ok(());
        };
        let name = function_name(funcls, it, def.span)?;
        let TAnnotOpt::Some { quant, typ } = annot else {
            return Ok(());
        };
        let history = self
            .values
            .get(name)
            .ok_or_else(|| TypeError::unbound_var(def.span, name.to_owned()))?;
        let index = history.partition_point(|b| b.position <= position);
        let binding = index
            .checked_sub(1)
            .map(|i| &history[i])
            .ok_or_else(|| TypeError::unbound_var(def.span, name.to_owned()))?;
        let scheme = &binding.value.scheme;
        if binding.value.inline && binding.position == position {
            if quant.len() != scheme.typschm.quant.len()
                || !quant
                    .iter()
                    .zip(&scheme.typschm.quant)
                    .all(|(a, b)| match (a, b) {
                        (QuantItem::Id(a), QuantItem::Id(b)) => {
                            a.kind == b.kind && it.resolve(a.kid) == scheme.interner.resolve(b.kid)
                        }
                        (QuantItem::Constraint(a), QuantItem::Constraint(b)) => {
                            crate::canonical_eq::constraint(a, b, it, &scheme.interner, true)
                        }
                        _ => false,
                    })
            {
                return Err(failure(
                    def.span,
                    "inline quantifiers do not match the synthesized signature",
                ));
            }
        } else if !quant.is_empty() {
            return Err(failure(
                def.span,
                "duplicate quantifier between inline annotation and val declaration",
            ));
        }
        self.check_annotation(typ, it, position, Some(scheme))?;
        let TypKind::Fn { ret, .. } = &scheme.typschm.typ.kind else {
            return Err(failure(def.span, "function signature is not an arrow type"));
        };
        if !crate::canonical_eq::typ(typ, ret, it, &scheme.interner, true) {
            return Err(failure(typ.span,"inline return type does not match the function signature (synonym expansion may be required)"));
        }
        Ok(())
    }

    /// Validate an expression/pattern annotation in its function's quantified
    /// scope. Interners are definition-local, so outer binders match by name.
    pub(crate) fn check_annotation(
        &self,
        typ: &Typ,
        it: &SimpleInterner,
        position: usize,
        scope: Option<&Scheme>,
    ) -> TypeResult<()> {
        let mut checker = WellFormed::new(self, it, position);
        if let Some(scope) = scope {
            for item in &scope.typschm.quant {
                if let QuantItem::Id(binder) = item {
                    checker.outer.insert(
                        text(scope.interner.as_ref(), binder.kid, typ.span)?.to_owned(),
                        binder.kind,
                    );
                    checker.bind_proof_name(
                        text(scope.interner.as_ref(), binder.kid, typ.span)?.to_owned(),
                    );
                }
            }
            for item in &scope.typschm.quant {
                if let QuantItem::Constraint(c) = item {
                    checker.assumptions.push(Assumption {
                        constraint: c,
                        interner: &scope.interner,
                        variables: checker.proof_vars.clone(),
                    });
                }
            }
        }
        checker.run(vec![Task::Typ(typ)])
    }

    /// Source-position lookup, safe to call concurrently after declarations
    /// are frozen. A worker never sees a later replacement or declaration.
    #[must_use]
    pub fn value(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.values
            .get(name)
            .and_then(|history| at(history, position))
            .map(|value| &value.scheme)
    }
    pub(crate) fn type_synonym(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.constructors
            .get(name)
            .and_then(|history| at(history, position))
            .and_then(|constructor| constructor.synonym.as_ref())
    }
    fn overload(&self, name: &str, position: usize) -> Option<&[String]> {
        self.overloads
            .get(name)
            .and_then(|history| at(history, position))
            .map(Vec::as_slice)
    }
    /// Whether an overload declaration is visible (members may dangle,
    /// e.g. backend builtins like `__id`; OCaml tolerates those too and
    /// only errors when no overload exists at all).
    pub(crate) fn has_overload(&self, name: &str, position: usize) -> bool {
        self.overload(name, position).is_some()
    }
    pub(crate) fn overload_candidates(
        &self,
        name: &str,
        position: usize,
    ) -> TypeResult<Option<Vec<Arc<Scheme>>>> {
        let Some(initial) = self.overload(name, position) else {
            return Ok(None);
        };
        enum Work<'a> {
            Enter(&'a str),
            Leave(&'a str),
        }
        let mut active = std::collections::BTreeSet::new();
        active.insert(name);
        let mut work: Vec<_> = initial
            .iter()
            .rev()
            .map(|n| Work::Enter(n.as_str()))
            .collect();
        let mut candidates = Vec::new();
        while let Some(task) = work.pop() {
            match task {
                Work::Leave(name) => {
                    active.remove(name);
                }
                Work::Enter(name) => {
                    if let Some(nested) = self.overload(name, position) {
                        if !active.insert(name) {
                            return Err(failure(Span::DUMMY, format!("cyclic overload '{name}'")));
                        }
                        work.push(Work::Leave(name));
                        work.extend(nested.iter().rev().map(|n| Work::Enter(n.as_str())));
                    } else {
                        let scheme = self
                            .register(name, position)
                            .or_else(|| self.value(name, position))
                            .ok_or_else(|| TypeError::unbound_var(Span::DUMMY, name.to_owned()))?;
                        candidates.push(scheme.clone());
                    }
                }
            }
        }
        Ok(Some(candidates))
    }
    pub(crate) fn enumeration_member(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.values
            .get(name)
            .and_then(|history| at(history, position))
            .filter(|value| value.enumeration_member)
            .map(|value| &value.scheme)
    }
    #[must_use]
    pub fn register(&self, name: &str, position: usize) -> Option<&Arc<Scheme>> {
        self.registers
            .get(name)
            .and_then(|history| at(history, position))
    }

    /// Source-position `struct` lookup (OCaml `Env.get_record`). Returns
    /// the preserved quantified telescope plus every field payload, or
    /// `None` when the struct is not declared at `position`.
    #[must_use]
    pub fn record(&self, name: &str, position: usize) -> Option<&RecordDef> {
        self.records
            .get(name)
            .and_then(|history| at(history, position))
    }

    /// Source-position `union` lookup (OCaml `Env.get_variant`). Returns
    /// the preserved quantified telescope plus every constructor
    /// payload, or `None` when the union is not declared at `position`.
    #[must_use]
    pub fn variant(&self, name: &str, position: usize) -> Option<&UnionDef> {
        self.unions
            .get(name)
            .and_then(|history| at(history, position))
    }

    /// Source-position field-accessor lookup (OCaml `Env.get_accessor`):
    /// the preserved `Fn(record) -> field` scheme for
    /// `record.field`, or `None` when the pair is not declared at
    /// `position`.
    #[must_use]
    pub fn accessor(
        &self,
        record: &str,
        field: &str,
        position: usize,
    ) -> Option<&Arc<Scheme>> {
        self.accessors
            .get(&(record.to_owned(), field.to_owned()))
            .and_then(|history| at(history, position))
    }

    /// Sorted struct names visible at `position` (deterministic order).
    #[must_use]
    pub fn record_names(&self, position: usize) -> Vec<String> {
        self.records
            .iter()
            .filter(|(_, history)| {
                history
                    .iter()
                    .rfind(|binding| binding.position <= position)
                    .is_some()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Sorted union names visible at `position` (deterministic order).
    #[must_use]
    pub fn variant_names(&self, position: usize) -> Vec<String> {
        self.unions
            .iter()
            .filter(|(_, history)| {
                history
                    .iter()
                    .rfind(|binding| binding.position <= position)
                    .is_some()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Source-position mapping lookup (OCaml `Env.is_mapping` /
    /// `get_val_spec` on the `Typ_bidir` scheme). Returns the preserved
    /// quantifier telescope plus the bidirectional type, or `None` when
    /// the mapping is not declared at `position`.
    #[must_use]
    pub fn mapping(&self, name: &str, position: usize) -> Option<&MappingDef> {
        self.mappings
            .get(name)
            .and_then(|history| at(history, position))
    }

    /// Whether `name` is a declared mapping at `position` (gates the
    /// `<m>_forwards` / `<m>_backwards` application expansion).
    #[must_use]
    pub fn is_mapping(&self, name: &str, position: usize) -> bool {
        self.mapping(name, position).is_some()
    }

    /// Generated clause-function names for a mapping application
    /// (mirrors the OCaml `mk_id (name ^ "_forwards")` convention used by
    /// the `E_app` mapping arms). Returns `None` when `name` is not a
    /// mapping at `position`, so the insertion only rewrites declared
    /// mappings and never plain or overloaded functions.
    #[must_use]
    pub fn mapping_expansion(&self, name: &str, position: usize) -> Option<(String, String)> {
        self.is_mapping(name, position).then(|| {
            (
                format!("{name}_forwards"),
                format!("{name}_backwards"),
            )
        })
    }

    /// Sorted mapping names visible at `position` (deterministic order).
    #[must_use]
    pub fn mapping_names(&self, position: usize) -> Vec<String> {
        self.mappings
            .iter()
            .filter(|(_, history)| {
                history
                    .iter()
                    .rfind(|binding| binding.position <= position)
                    .is_some()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Source-position `scattered` block lookup: the latest opening (or
    /// closing) declaration of `(namespace, name)` at `position`.
    #[must_use]
    pub fn scattered(
        &self,
        namespace: ScatteredNamespace,
        name: &str,
        position: usize,
    ) -> Option<&ScatteredDecl> {
        self.scattered
            .get(&(namespace, name.to_owned()))
            .and_then(|history| at(history, position))
    }

    /// Clauses recorded for a scattered block at `position`
    /// (`SD_funcl` / `SD_mapcl` / `SD_unioncl` / `SD_enumcl` counts).
    #[must_use]
    pub fn scattered_clause_count(
        &self,
        namespace: ScatteredNamespace,
        name: &str,
        position: usize,
    ) -> usize {
        self.scattered_clauses
            .get(&(namespace, name.to_owned()))
            .and_then(|history| at(history, position))
            .copied()
            .unwrap_or(0)
    }

    /// Namespace closed by the `scattered end` declared at exactly
    /// `position`, if any (Lane-37 linkage for the body pass: the end's own
    /// declare entry is visible at body time and masks the open block).
    #[must_use]
    pub fn scattered_closed_at(&self, name: &str, position: usize) -> Option<ScatteredNamespace> {
        [
            ScatteredNamespace::Function,
            ScatteredNamespace::Mapping,
            ScatteredNamespace::Variant,
            ScatteredNamespace::Enum,
        ]
        .into_iter()
        .find(|ns| {
            self.scattered.get(&(*ns, name.to_owned())).is_some_and(|history| {
                history
                    .iter()
                    .rfind(|b| b.position <= position)
                    .is_some_and(|b| b.position == position && b.value.closed)
            })
        })
    }

    /// Source-position outcome lookup (OCaml `Env` outcome entry):
    /// parameters plus result scheme, or `None` when the outcome is not
    /// declared at `position`.
    #[must_use]
    pub fn outcome(&self, name: &str, position: usize) -> Option<&OutcomeDef> {
        self.outcomes
            .get(name)
            .and_then(|history| at(history, position))
    }

    /// Sorted outcome names visible at `position` (deterministic order).
    #[must_use]
    pub fn outcome_names(&self, position: usize) -> Vec<String> {
        self.outcomes
            .iter()
            .filter(|(_, history)| {
                history
                    .iter()
                    .rfind(|binding| binding.position <= position)
                    .is_some()
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Outcome-local `val` scheme (OCaml outcome-local environment):
    /// `(outcome, val)` at `position`, never the global table.
    #[must_use]
    pub fn outcome_val(
        &self,
        outcome: &str,
        name: &str,
        position: usize,
    ) -> Option<&Arc<Scheme>> {
        self.outcome_vals
            .get(&(outcome.to_owned(), name.to_owned()))
            .and_then(|history| at(history, position))
    }

    /// Latest `impl` target before or at `position`, if any.
    #[must_use]
    pub fn last_impl(&self, position: usize) -> Option<&str> {
        let index = self.impls.partition_point(|b| b.position <= position);
        index
            .checked_sub(1)
            .map(|index| self.impls[index].value.as_str())
    }

    /// Solver-Beweiszähler des internen Pools (für den CLI-Profiler;
    /// Lane-35-Hook, Lane-15-Zähler).
    #[must_use]
    pub fn smt_totals(&self) -> (u64, f64) {
        self.solvers.smt_totals()
    }

    /// Termination-measure presence for a function at `position`
    /// (`DEF_measure` span); the pattern/body type via
    /// `exp_arms::check_measure` in the body pass.
    #[must_use]
    pub fn measure(&self, name: &str, position: usize) -> Option<Span> {
        self.measures
            .get(name)
            .and_then(|history| at(history, position))
            .copied()
    }

    /// Loop-measure presence for a function at `position`.
    #[must_use]
    pub fn loop_measure(&self, name: &str, position: usize) -> Option<Span> {
        self.loop_measures
            .get(name)
            .and_then(|history| at(history, position))
            .copied()
    }

    /// Whether `outcome` was instantiated at or before `position`.
    #[must_use]
    pub fn is_instantiated(&self, outcome: &str, position: usize) -> bool {
        let index = self
            .instantiations
            .partition_point(|b| b.position <= position);
        self.instantiations[..index]
            .iter()
            .any(|b| b.value.0 == outcome)
    }


    fn signature(&self, name: &str, position: usize) -> Option<Constructor> {
        self.constructors
            .get(name)
            .and_then(|history| at(history, position))
            .cloned()
            .or_else(|| builtin(name))
    }
}

fn arg_kind(arg: &TypArg) -> Kind {
    match arg.kind {
        TypArgKind::NExp(_) => Kind::Int,
        TypArgKind::Typ(_) => Kind::Type,
        TypArgKind::Bool(_) => Kind::Bool,
    }
}
fn function_name<'a>(clauses: &[Funcl], it: &'a SimpleInterner, span: Span) -> TypeResult<&'a str> {
    let first = clauses
        .first()
        .ok_or_else(|| failure(span, "function has no clauses"))?;
    let name = text(it, first.name.symbol, first.span)?;
    for clause in clauses.iter().skip(1) {
        if text(it, clause.name.symbol, clause.span)? != name {
            return Err(failure(
                clause.span,
                "all clauses in a function definition must have the same name",
            ));
        }
    }
    Ok(name)
}
fn infer_pattern_type(pat: &Pat, it: &mut SimpleInterner) -> TypeResult<Typ> {
    enum Work<'a> {
        Pattern(&'a Pat),
        Tuple(Span, usize),
    }
    let mut work = vec![Work::Pattern(pat)];
    struct Types(Vec<Typ>);
    impl Drop for Types {
        fn drop(&mut self) {
            dispose_signature(
                vec![],
                Node::dummy(TypKind::Tuple(std::mem::take(&mut self.0))),
            );
        }
    }
    let mut types = Types(Vec::new());
    while let Some(task) = work.pop() {
        match task {
            Work::Pattern(p) => match &p.kind {
                PatKind::Typ { typ, .. } => types.0.push(typ.clone()),
                PatKind::Lit(lit) => types
                    .0
                    .push(crate::canonical_literal::classify(lit, it)?.into_type(it)),
                PatKind::Tuple(items) => {
                    work.push(Work::Tuple(p.span, items.len()));
                    work.extend(items.iter().rev().map(Work::Pattern));
                }
                _ => {
                    return Err(failure(
                        p.span,
                        "cannot infer an argument type from an unannotated pattern",
                    ))
                }
            },
            Work::Tuple(span, count) => {
                let items = types.0.split_off(types.0.len() - count);
                types
                    .0
                    .push(Node::new(NodeId::DUMMY, span, TypKind::Tuple(items)));
            }
        }
    }
    Ok(types.0.pop().expect("inferred root pattern type"))
}
enum Task<'a> {
    Typ(&'a Typ),
    Arg(&'a TypArg, Kind),
    NExp(&'a NExp),
    Constraint(&'a Constraint),
    Assume(&'a Constraint),
    Range(&'a IndexRange),
    Discharge(&'a Name, &'a [TypArg], Constructor),
    Restore(Vec<(Symbol, Option<Kind>)>, BTreeMap<String, u64>, usize),
}
struct Assumption<'a> {
    constraint: &'a Constraint,
    interner: &'a SimpleInterner,
    variables: BTreeMap<String, u64>,
}
struct WellFormed<'a> {
    program: &'a Program,
    it: &'a SimpleInterner,
    position: usize,
    vars: BTreeMap<Symbol, Kind>,
    outer: BTreeMap<String, Kind>,
    in_existential: bool,
    proof_vars: BTreeMap<String, u64>,
    next_proof_var: u64,
    assumptions: Vec<Assumption<'a>>,
}
impl<'a> WellFormed<'a> {
    fn new(program: &'a Program, it: &'a SimpleInterner, position: usize) -> Self {
        Self {
            program,
            it,
            position,
            vars: BTreeMap::new(),
            outer: BTreeMap::new(),
            in_existential: false,
            proof_vars: BTreeMap::new(),
            next_proof_var: 0,
            assumptions: Vec::new(),
        }
    }
    fn bind_proof_name(&mut self, name: String) {
        let id = self.next_proof_var;
        self.next_proof_var += 1;
        self.proof_vars.insert(name, id);
    }
    fn quant(&mut self, quant: &'a [QuantItem]) -> TypeResult<()> {
        for item in quant {
            if let QuantItem::Id(binder) = item {
                self.bind_proof_name(text(self.it, binder.kid, Span::DUMMY)?.to_owned());
                if self.vars.insert(binder.kid, binder.kind).is_some() {
                    return Err(failure(
                        Span::DUMMY,
                        format!(
                            "duplicate quantified variable {}",
                            text(self.it, binder.kid, Span::DUMMY)?
                        ),
                    ));
                }
            }
        }
        self.run(
            quant
                .iter()
                .filter_map(|item| match item {
                    QuantItem::Id(_) => None,
                    QuantItem::Constraint(c) => Some(Task::Constraint(c)),
                })
                .collect(),
        )?;
        for item in quant {
            if let QuantItem::Constraint(c) = item {
                self.assumptions.push(Assumption {
                    constraint: c,
                    interner: self.it,
                    variables: self.proof_vars.clone(),
                });
            }
        }
        Ok(())
    }
    fn variable(&self, symbol: Symbol, expected: Kind, span: Span) -> TypeResult<()> {
        let name = text(self.it, symbol, span)?;
        match self.vars.get(&symbol).or_else(|| self.outer.get(name)) {
            Some(found) if *found == expected => Ok(()),
            Some(found) => Err(failure(
                span,
                format!("kind mismatch for {name}: expected {expected:?}, found {found:?}"),
            )),
            None => Err(TypeError::unbound_var(span, name.to_owned())),
        }
    }
    fn application(&self, name: &Name, arity: usize, result: Kind) -> TypeResult<Constructor> {
        let name_text = text(self.it, name.symbol, name.span)?;
        let signature = self
            .program
            .signature(name_text, self.position)
            .ok_or_else(|| TypeError::unbound_var(name.span, name_text.to_owned()))?;
        if signature.result != result {
            return Err(failure(
                name.span,
                format!(
                    "constructor {name_text} returns {:?}, expected {result:?}",
                    signature.result
                ),
            ));
        }
        if signature.args.len() != arity {
            return Err(TypeError {
                span: name.span,
                kind: TypeErrorKind::ArityMismatch {
                    expected: signature.args.len(),
                    found: arity,
                },
            });
        }
        Ok(signature)
    }
    fn discharge(&self, name: &Name, args: &[TypArg], signature: &Constructor) -> TypeResult<()> {
        let Some(proof) = &signature.proof else {
            return Ok(());
        };
        let context = Context {
            interner: self.it,
            variables: &self.proof_vars,
            bindings: None,
        };
        let substitution = Substitution {
            arguments: proof
                .quant
                .iter()
                .filter_map(|q| {
                    if let QuantItem::Id(k) = q {
                        Some(k.kid)
                    } else {
                        None
                    }
                })
                .zip(args)
                .collect(),
            context,
        };
        let empty = BTreeMap::new();
        let mut assumptions: Vec<_> = self
            .assumptions
            .iter()
            .map(|a| View {
                negated: false,
                constraint: a.constraint,
                context: Context {
                    interner: a.interner,
                    variables: &a.variables,
                    bindings: None,
                },
            })
            .collect();
        assumptions.extend(
            self.program
                .constraints
                .iter()
                .filter(|b| b.position <= self.position)
                .map(|b| View {
                    negated: false,
                    constraint: &b.value.0,
                    context: Context {
                        interner: &b.value.1,
                        variables: &empty,
                        bindings: None,
                    },
                }),
        );
        for item in &proof.quant {
            let QuantItem::Constraint(c) = item else {
                continue;
            };
            let query = CanonicalQuery::implication(
                &assumptions,
                View {
                    negated: false,
                    constraint: c,
                    context: Context {
                        interner: &proof.interner,
                        variables: &empty,
                        bindings: None,
                    },
                },
                Some(&substitution),
            )
            .map_err(|reason| TypeError {
                span: name.span,
                kind: TypeErrorKind::Unsupported(reason),
            })?;
            let verdict = self.program.prove(query);
            if !verdict.is_valid() {
                return Err(TypeError::constraint_not_proved(
                    name.span,
                    format!(
                        "type constructor {} ({verdict:?})",
                        text(self.it, name.symbol, name.span)?
                    ),
                ));
            }
        }
        Ok(())
    }
    fn run(&mut self, mut work: Vec<Task<'a>>) -> TypeResult<()> {
        while let Some(task) = work.pop() {
            match task {
                Task::Range(range) => match &range.kind {
                    IndexRangeKind::Single(n) => work.push(Task::NExp(n)),
                    IndexRangeKind::Range { lo, hi } => {
                        work.push(Task::NExp(hi));
                        work.push(Task::NExp(lo));
                    }
                    IndexRangeKind::Concat(a, b) => {
                        work.push(Task::Range(b));
                        work.push(Task::Range(a));
                    }
                },
                Task::Typ(typ) => match &typ.kind {
                    TypKind::Unknown => {
                        return Err(failure(typ.span, "unresolved type placeholder"))
                    }
                    TypKind::Id(name) => {
                        let signature = self.application(name, 0, Kind::Type)?;
                        self.discharge(name, &[], &signature)?;
                    }
                    TypKind::Var(symbol) => self.variable(*symbol, Kind::Type, typ.span)?,
                    TypKind::Fn { args, ret } => {
                        work.push(Task::Typ(ret));
                        work.extend(args.iter().map(Task::Typ));
                    }
                    TypKind::Bidir { lhs, rhs } => {
                        work.push(Task::Typ(rhs));
                        work.push(Task::Typ(lhs));
                    }
                    TypKind::Tuple(items) => work.extend(items.iter().map(Task::Typ)),
                    TypKind::App { name, args } => {
                        let signature = self.application(name, args.len(), Kind::Type)?;
                        work.push(Task::Discharge(name, args, signature.clone()));
                        work.extend(
                            args.iter()
                                .zip(signature.args)
                                .map(|(arg, kind)| Task::Arg(arg, kind)),
                        );
                    }
                    TypKind::Exist {
                        binders,
                        constraint,
                        typ,
                    } => {
                        if binders.is_empty() || self.in_existential {
                            return Err(failure(
                                typ.span,
                                "existentials require binders and cannot nest",
                            ));
                        }
                        let mut names = std::collections::BTreeSet::new();
                        if binders.iter().any(|binder| !names.insert(binder.kid)) {
                            return Err(failure(typ.span, "duplicate existential binder"));
                        }
                        self.in_existential = true;
                        let old_vars = self.proof_vars.clone();
                        let old_assumptions = self.assumptions.len();
                        for binder in binders {
                            self.bind_proof_name(text(self.it, binder.kid, typ.span)?.to_owned());
                        }
                        let restored = binders
                            .iter()
                            .map(|binder| (binder.kid, self.vars.insert(binder.kid, binder.kind)))
                            .collect();
                        work.push(Task::Restore(restored, old_vars, old_assumptions));
                        work.push(Task::Typ(typ));
                        work.push(Task::Assume(constraint));
                        work.push(Task::Constraint(constraint));
                    }
                },
                Task::Arg(arg, expected) => {
                    let found = arg_kind(arg);
                    if found != expected {
                        return Err(failure(arg.span, format!("type argument kind mismatch: expected {expected:?}, found {found:?}")));
                    }
                    work.push(match &arg.kind {
                        TypArgKind::Typ(t) => Task::Typ(t),
                        TypArgKind::NExp(n) => Task::NExp(n),
                        TypArgKind::Bool(c) => Task::Constraint(c),
                    });
                }
                Task::NExp(n) => match &n.kind {
                    NExpKind::Constant(_) => {}
                    NExpKind::Var(symbol) => self.variable(*symbol, Kind::Int, n.span)?,
                    NExpKind::Id(name) => {
                        let signature = self.application(name, 0, Kind::Int)?;
                        self.discharge(name, &[], &signature)?;
                    }
                    NExpKind::App { func, args } => {
                        let signature = self.application(func, args.len(), Kind::Int)?;
                        if signature.proof.is_some() {
                            return Err(TypeError {
                                span: func.span,
                                kind: TypeErrorKind::Unsupported(
                                    "constrained numeric application".into(),
                                ),
                            });
                        }
                        if signature.args.iter().any(|kind| *kind != Kind::Int) {
                            return Err(failure(
                                n.span,
                                "numeric application argument kind mismatch",
                            ));
                        }
                        work.extend(args.iter().map(Task::NExp));
                    }
                    NExpKind::Times(a, b) | NExpKind::Sum(a, b) | NExpKind::Minus(a, b) => {
                        work.push(Task::NExp(b));
                        work.push(Task::NExp(a));
                    }
                    NExpKind::Exp(n) | NExpKind::Neg(n) => work.push(Task::NExp(n)),
                    NExpKind::If {
                        cond,
                        then_branch,
                        else_branch,
                    } => {
                        work.push(Task::NExp(else_branch));
                        work.push(Task::NExp(then_branch));
                        work.push(Task::Constraint(cond));
                    }
                },
                Task::Constraint(c) => match &c.kind {
                    ConstraintKind::True | ConstraintKind::False => {}
                    ConstraintKind::Id(name) => {
                        let signature = self.application(name, 0, Kind::Bool)?;
                        self.discharge(name, &[], &signature)?;
                    }
                    ConstraintKind::Var(symbol) => self.variable(*symbol, Kind::Bool, c.span)?,
                    ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
                        let kind = arg_kind(a);
                        work.push(Task::Arg(b, kind));
                        work.push(Task::Arg(a, kind));
                    }
                    ConstraintKind::Ge(a, b)
                    | ConstraintKind::Gt(a, b)
                    | ConstraintKind::Le(a, b)
                    | ConstraintKind::Lt(a, b) => {
                        work.push(Task::NExp(b));
                        work.push(Task::NExp(a));
                    }
                    ConstraintKind::Set { nexp, .. } => work.push(Task::NExp(nexp)),
                    ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
                        work.push(Task::Constraint(b));
                        work.push(Task::Constraint(a));
                    }
                    ConstraintKind::Not(c) => work.push(Task::Constraint(c)),
                    ConstraintKind::App { func, args } => {
                        let signature = self.application(func, args.len(), Kind::Bool)?;
                        work.push(Task::Discharge(func, args, signature.clone()));
                        work.extend(
                            args.iter()
                                .zip(signature.args)
                                .map(|(arg, kind)| Task::Arg(arg, kind)),
                        );
                    }
                },
                Task::Assume(constraint) => self.assumptions.push(Assumption {
                    constraint,
                    interner: self.it,
                    variables: self.proof_vars.clone(),
                }),
                Task::Discharge(name, args, signature) => self.discharge(name, args, &signature)?,
                Task::Restore(vars, proof_vars, assumptions) => {
                    self.proof_vars = proof_vars;
                    self.assumptions.truncate(assumptions);
                    self.in_existential = false;
                    for (symbol, previous) in vars.into_iter().rev() {
                        if let Some(kind) = previous {
                            self.vars.insert(symbol, kind);
                        } else {
                            self.vars.remove(&symbol);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
