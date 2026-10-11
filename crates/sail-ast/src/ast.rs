#![forbid(unsafe_code)]
//! Sail2 core AST data types, shaped after `sail/src/lib/extraction/Ast.ml`
//! (structure only, no OCaml logic).
//!
//! Every major node is [`Node<K>`]: a generational [`NodeId`] plus a
//! [`Span`] plus a `kind` enum. Leaves that never need identity
//! (`KOpt`, `TypSchm`, `QuantItem`, ...) are plain structs/enums.

use crate::{NodeId, Span, Symbol};
use serde::{Deserialize, Serialize};

/// A located AST node: generational identity + source span + payload.
/// `Send + Sync` whenever `K` is (all our `K`s are).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node<K> {
    pub id: NodeId,
    pub span: Span,
    pub kind: K,
}

impl<K> Node<K> {
    #[must_use]
    pub fn new(id: NodeId, span: Span, kind: K) -> Self {
        Node { id, span, kind }
    }

    #[must_use]
    pub fn dummy(kind: K) -> Self {
        Node {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind,
        }
    }
}

// ---------------------------------------------------------------------------
// Names, kinds, literals
// ---------------------------------------------------------------------------

/// An identifier occurrence. `is_operator` distinguishes `Operator "++"`
/// from plain `Id "foo"` in the OCaml AST.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Name {
    pub id: NodeId,
    pub span: Span,
    pub symbol: Symbol,
    pub is_operator: bool,
}

impl Name {
    #[must_use]
    pub fn new(id: NodeId, span: Span, symbol: Symbol, is_operator: bool) -> Self {
        Name {
            id,
            span,
            symbol,
            is_operator,
        }
    }

    /// Synthesized name for conversions and desugaring (dummy id/span,
    /// non-operator). Prefer a real span when one is available.
    #[must_use]
    pub fn dummy(symbol: Symbol) -> Self {
        Name {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            symbol,
            is_operator: false,
        }
    }
}

/// Sail kinds: `Type`, `Int`, `Bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Type,
    Int,
    Bool,
}

/// A kind-annotated type-level variable binder (`KOpt_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KOpt {
    pub kind: Kind,
    pub kid: Symbol,
}

/// Literals (`lit_aux`). Payloads are interned [`Symbol`]s resolved
/// through the caller's [`Interner`][crate::Interner]: repeated literals
/// share one table entry instead of one `String` per occurrence (Lane 71
/// payload-intern). JSON snapshots therefore carry numeric payloads (see
/// `docs/payload-intern-contract.md`).
pub type Lit = Node<LitKind>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LitKind {
    Unit,
    True,
    False,
    /// Decimal integer literal, e.g. `"42"`, `"-7"`.
    Num(Symbol),
    /// Hex digits without `0x` prefix, e.g. `"FF"`.
    Hex(Symbol),
    /// Binary digits without `0b` prefix, e.g. `"101"`.
    Bin(Symbol),
    String(Symbol),
    /// Real literal in decimal notation, e.g. `"1.5"`.
    Real(Symbol),
}

// ---------------------------------------------------------------------------
// Numeric expressions, types, constraints
// ---------------------------------------------------------------------------

/// Numeric/size expressions (`nexp_aux`).
pub type NExp = Node<NExpKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NExpKind {
    Id(Name),
    /// Type-level variable (`Nexp_var`).
    Var(Symbol),
    /// Decimal integer constant, e.g. `"8"`.
    Constant(String),
    App { func: Name, args: Vec<NExp> },
    If {
        cond: Box<Constraint>,
        then_branch: Box<NExp>,
        else_branch: Box<NExp>,
    },
    Times(Box<NExp>, Box<NExp>),
    Sum(Box<NExp>, Box<NExp>),
    Minus(Box<NExp>, Box<NExp>),
    Exp(Box<NExp>),
    Neg(Box<NExp>),
}

/// Type arguments (`typ_arg_aux`).
pub type TypArg = Node<TypArgKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypArgKind {
    NExp(NExp),
    Typ(Typ),
    Bool(Box<Constraint>),
}

/// Types (`typ_aux`).
pub type Typ = Node<TypKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypKind {
    /// Placeholder the typechecker fills in later.
    Unknown,
    Id(Name),
    Var(Symbol),
    Fn { args: Vec<Typ>, ret: Box<Typ> },
    Bidir { lhs: Box<Typ>, rhs: Box<Typ> },
    Tuple(Vec<Typ>),
    App { name: Name, args: Vec<TypArg> },
    Exist {
        binders: Vec<KOpt>,
        constraint: Box<Constraint>,
        typ: Box<Typ>,
    },
}

/// Numeric constraints (`n_constraint_aux`).
pub type Constraint = Node<ConstraintKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstraintKind {
    Equal(Box<TypArg>, Box<TypArg>),
    NotEqual(Box<TypArg>, Box<TypArg>),
    Ge(NExp, NExp),
    Gt(NExp, NExp),
    Le(NExp, NExp),
    Lt(NExp, NExp),
    Set { nexp: NExp, members: Vec<String> },
    And(Box<Constraint>, Box<Constraint>),
    Or(Box<Constraint>, Box<Constraint>),
    Not(Box<Constraint>),
    App { func: Name, args: Vec<TypArg> },
    Id(Name),
    Var(Symbol),
    True,
    False,
}

/// Quantifier items and type schemes (`quant_item`, `typschm`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuantItem {
    Id(KOpt),
    Constraint(Constraint),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypSchm {
    pub quant: Vec<QuantItem>,
    pub typ: Typ,
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructName {
    Id(Name),
    Anon,
}

/// Type-level patterns (`typ_pat_aux`).
pub type TypPat = Node<TypPatKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypPatKind {
    Wild,
    Var(Symbol),
    App { name: Name, args: Vec<TypPat> },
}

/// Value patterns (`pat_aux`, untyped annotation = `()`).
pub type Pat = Node<PatKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatKind {
    Lit(Lit),
    Wild,
    Or(Box<Pat>, Box<Pat>),
    Not(Box<Pat>),
    As { pat: Box<Pat>, id: Name },
    Typ { typ: Typ, pat: Box<Pat> },
    Id(Name),
    Var { pat: Box<Pat>, typ_pat: TypPat },
    App { func: Name, args: Vec<Pat> },
    Vector(Vec<Pat>),
    VectorConcat(Vec<Pat>),
    VectorSubrange { id: Name, lo: String, hi: String },
    Tuple(Vec<Pat>),
    List(Vec<Pat>),
    Cons(Box<Pat>, Box<Pat>),
    StringAppend(Vec<Pat>),
    Struct {
        name: StructName,
        fields: Vec<(Name, Pat)>,
        wildcard: bool,
    },
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Loop {
    While,
    Until,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Order {
    Inc,
    Dec,
}

/// Internal values for `E_internal_value` (test/snapshot use).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Unit,
    Bool(bool),
    /// Decimal integer.
    Int(String),
    String(String),
    Tuple(Vec<Value>),
    List(Vec<Value>),
}

/// Left-hand-side expressions (`lexp_aux`).
pub type LExp = Node<LExpKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LExpKind {
    Id(Name),
    Deref(Box<Exp>),
    Typ { typ: Typ, id: Name },
    Tuple(Vec<LExp>),
    VectorConcat(Vec<LExp>),
    Vector { lexp: Box<LExp>, index: Box<Exp> },
    VectorRange {
        lexp: Box<LExp>,
        lo: Box<Exp>,
        hi: Box<Exp>,
    },
    Field { lexp: Box<LExp>, field: Name },
}

/// Field initializers (`fexp`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FExp {
    pub id: NodeId,
    pub span: Span,
    pub field: Name,
    pub value: Exp,
}

/// Match arms (`pexp`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PExp {
    pub id: NodeId,
    pub span: Span,
    pub kind: PExpKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PExpKind {
    PatExp { pat: Pat, body: Exp },
    PatWhen { pat: Pat, guard: Exp, body: Exp },
}

/// Expressions (`exp_aux`, untyped annotation = `()`).
pub type Exp = Node<ExpKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpKind {
    Block(Vec<Exp>),
    Id(Name),
    Lit(Lit),
    Typ { typ: Typ, exp: Box<Exp> },
    App { func: Name, args: Vec<Exp> },
    Tuple(Vec<Exp>),
    If {
        cond: Box<Exp>,
        then_branch: Box<Exp>,
        else_branch: Box<Exp>,
    },
    Loop {
        loop_kind: Loop,
        measure: Option<Box<Exp>>,
        cond: Box<Exp>,
        body: Box<Exp>,
    },
    For {
        var: Name,
        from: Box<Exp>,
        to: Box<Exp>,
        step: Box<Exp>,
        order: Order,
        body: Box<Exp>,
    },
    Vector(Vec<Exp>),
    VectorAppend(Box<Exp>, Box<Exp>),
    List(Vec<Exp>),
    Cons(Box<Exp>, Box<Exp>),
    Struct {
        name: StructName,
        fields: Vec<FExp>,
    },
    StructUpdate {
        record: Box<Exp>,
        fields: Vec<FExp>,
    },
    Field { record: Box<Exp>, field: Name },
    Match {
        scrutinee: Box<Exp>,
        arms: Vec<PExp>,
    },
    Let {
        pat: Pat,
        bind: Box<Exp>,
        body: Box<Exp>,
    },
    Assign { lexp: Box<LExp>, value: Box<Exp> },
    Sizeof(NExp),
    Return(Box<Exp>),
    Exit(Box<Exp>),
    /// `%config` keys.
    Config(Vec<String>),
    Ref(Name),
    Throw(Box<Exp>),
    Try { exp: Box<Exp>, arms: Vec<PExp> },
    Assert { cond: Box<Exp>, msg: Box<Exp> },
    Var {
        lexp: Box<LExp>,
        init: Box<Exp>,
        body: Box<Exp>,
    },
    Undef,
    InternalPlet {
        pat: Pat,
        bind: Box<Exp>,
        body: Box<Exp>,
    },
    InternalReturn(Box<Exp>),
    InternalValue(Value),
    InternalAssume {
        constraint: Constraint,
        exp: Box<Exp>,
    },
    Constraint(Constraint),
}

// ---------------------------------------------------------------------------
// Functions, mappings, type/register definitions
// ---------------------------------------------------------------------------

/// A single function clause (`funcl`): name + one match arm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Funcl {
    pub id: NodeId,
    pub span: Span,
    pub name: Name,
    pub arm: PExp,
}

/// A mapping clause (`mapcl`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapcl {
    pub id: NodeId,
    pub span: Span,
    pub kind: MapclKind,
}

/// One side of a bidirectional mapping (`mpexp_aux`). It is a mapping
/// pattern with an optional guard, not a match arm with a fabricated body.
pub type MPExp = Node<MPExpKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MPExpKind {
    Pat(Pat),
    When { pat: Pat, guard: Exp },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapclKind {
    Bidir { forwards: MPExp, backwards: MPExp },
    Forwards(PExp),
    Backwards(PExp),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecOpt {
    NonRec,
    Rec,
    Measure { pat: Pat, exp: Exp },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TAnnotOpt {
    None,
    Some { quant: Vec<QuantItem>, typ: Typ },
}

/// One union constructor (`type_union`: `typ id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeUnion {
    pub id: NodeId,
    pub span: Span,
    pub typ: Typ,
    pub ctor: Name,
}

/// Bitfield index ranges (`index_range_aux`).
pub type IndexRange = Node<IndexRangeKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexRangeKind {
    Single(NExp),
    Range { lo: NExp, hi: NExp },
    Concat(Box<IndexRange>, Box<IndexRange>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extern {
    pub pure: bool,
    pub bindings: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValSpec {
    pub id: NodeId,
    pub span: Span,
    pub name: Name,
    pub typschm: TypSchm,
    pub extern_opt: Option<Extern>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeDefKind {
    Abbrev {
        name: Name,
        quant: Vec<QuantItem>,
        arg: TypArg,
    },
    Record {
        name: Name,
        quant: Vec<QuantItem>,
        fields: Vec<(Name, Typ)>,
        wf: bool,
    },
    Variant {
        name: Name,
        quant: Vec<QuantItem>,
        ctors: Vec<TypeUnion>,
        wf: bool,
    },
    Enum {
        name: Name,
        members: Vec<Name>,
        wf: bool,
    },
    Abstract {
        name: Name,
        kind: Kind,
    },
    Bitfield {
        name: Name,
        typ: Typ,
        fields: Vec<(Name, IndexRange)>,
    },
}

pub type TypeDef = Node<TypeDefKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecSpec {
    pub id: NodeId,
    pub span: Span,
    pub typ: Typ,
    pub name: Name,
    pub init: Option<Exp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Fixity {
    Infix,
    InfixL,
    InfixR,
}

/// Type/value substitutions for instantiations (`subst_aux`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subst {
    Typ { kid: Symbol, arg: TypArg },
    Id { from: Name, to: Name },
}

// ---------------------------------------------------------------------------
// Top-level definitions
// ---------------------------------------------------------------------------

/// Scattered definitions (`scattered_def_aux`, untyped annotation = `()`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScatteredDef {
    Function { name: Name, annot: TAnnotOpt },
    Funcl(Funcl),
    Variant { name: Name, quant: Vec<QuantItem> },
    Unioncl { name: Name, union: TypeUnion },
    Mapping { name: Name, annot: TAnnotOpt },
    Mapcl { name: Name, clause: Mapcl },
    Enum(Name),
    Enumcl { enum_name: Name, member: Name },
    End(Name),
}

/// Top-level definitions (`def_aux`, untyped annotation = `()`).
pub type Def = Node<DefKind>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefKind {
    Type(TypeDef),
    Constraint(Constraint),
    Fundef {
        rec: RecOpt,
        annot: TAnnotOpt,
        funcls: Vec<Funcl>,
    },
    Mapdef {
        name: Name,
        annot: TAnnotOpt,
        clauses: Vec<Mapcl>,
    },
    Impl(Funcl),
    Let { pat: Pat, exp: Exp },
    Val(ValSpec),
    Outcome {
        name: Name,
        typschm: TypSchm,
        quant: Vec<QuantItem>,
        #[serde(default)]
        defs: Vec<Def>,
    },
    Instantiation {
        name: Name,
        substs: Vec<Subst>,
    },
    Fixity {
        fixity: Fixity,
        level: String,
        id: Name,
    },
    Overload {
        name: Name,
        overloads: Vec<Name>,
    },
    Default(Order),
    Scattered(ScatteredDef),
    Measure {
        name: Name,
        pat: Pat,
        exp: Exp,
    },
    LoopMeasures {
        name: Name,
        measures: Vec<(Loop, Exp)>,
    },
    Register(DecSpec),
    InternalMutrec(Vec<FunclSet>),
    Pragma { key: String, payload: String },
}

/// One function inside `DEF_internal_mutrec`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunclSet {
    pub rec: RecOpt,
    pub annot: TAnnotOpt,
    pub funcls: Vec<Funcl>,
}

/// A whole Sail file / snapshot root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ast {
    pub defs: Vec<Def>,
}

impl Ast {
    #[must_use]
    pub fn new(defs: Vec<Def>) -> Self {
        Ast { defs }
    }

    /// Serialize to JSON (snapshots for later lanes / tests).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        crate::json::to_string(self)
    }

    /// Deserialize from JSON produced by [`Ast::to_json`].
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        crate::json::from_str(text)
    }

    fn assert_send_sync()
    where
        Self: Send + Sync,
    {
    }
}

#[allow(dead_code)]
fn assert_all_send_sync() {
    fn is<T: Send + Sync>() {}
    is::<Name>();
    is::<KOpt>();
    is::<Lit>();
    is::<NExp>();
    is::<Typ>();
    is::<TypArg>();
    is::<Constraint>();
    is::<QuantItem>();
    is::<TypSchm>();
    is::<Pat>();
    is::<TypPat>();
    is::<Exp>();
    is::<LExp>();
    is::<FExp>();
    is::<PExp>();
    is::<Funcl>();
    is::<Mapcl>();
    is::<TypeDef>();
    is::<DecSpec>();
    is::<Def>();
    is::<Ast>();
    Ast::assert_send_sync();
}
