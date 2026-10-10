//! Skeletal Sail AST for the Menhir → LALRPOP grammar port.
//!
//! Mirrors the syntactic categories of `sail/src/lib/parser.mly`
//! (`Typ`/`Pat`/`Exp`/`Def` …) but stores raw surface syntax only:
//! no source locations, no desugaring, no deprecation handling.
//! Deviations from Menhir behaviour are listed in `GRAMMAR.md`.

#![forbid(unsafe_code)]

/// Sail literals (`lit` in `parser.mly`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lit {
    True,
    False,
    Unit,
    Num(String),
    Zero,
    One,
    Bin(String),
    Hex(String),
    Str(String),
    Real(String),
}

/// Kinds (`kind` in `parser.mly`): `Int`, `Nat`, `Type`, `Order`, `Bool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Int,
    Nat,
    Type,
    Order,
    Bool,
}

/// Kinded optionally-quantified variable (`kopt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KOpt {
    pub constant: bool,
    pub kids: Vec<String>,
    pub kind: Option<Kind>,
}

/// Type quantifier prefix (`typquant`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypQuant {
    pub kopts: Vec<KOpt>,
    pub constraint: Option<Box<Typ>>,
}

/// User-defined infix chain: prefix operators, head item, `(operator,
/// operator-local prefixes, item)` tail. No associativity is resolved here
/// (the OCaml side folds `E_infix`/`ATyp_infix` later).
#[derive(Debug, Clone, PartialEq)]
pub struct InfixChain<T> {
    pub prefixes: Vec<String>,
    pub head: T,
    pub tail: Vec<(String, Vec<String>, T)>,
}

/// Sail types (`typ` / `atomic_typ`).
#[derive(Debug, Clone, PartialEq)]
pub enum Typ {
    Id(String),
    Wild,
    Var(String),
    Lit(Lit),
    Dec,
    Inc,
    App(String, Vec<Typ>),
    Tuple(Vec<Typ>),
    Fn(Box<Typ>, Box<Typ>),
    NSet(Vec<String>),
    Exist(Vec<KOpt>, Box<Typ>, Box<Typ>),
    If(Box<Typ>, Box<Typ>, Box<Typ>),
    Infix(Box<InfixChain<Typ>>),
}

/// Function-type arrow: `->` or `<->` (`typschm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrow {
    Fn,
    Bidir,
}

/// Type scheme (`typschm`).
#[derive(Debug, Clone, PartialEq)]
pub struct TypSchm {
    pub quant: Option<TypQuant>,
    pub lhs: Typ,
    pub arrow: Arrow,
    pub rhs: Typ,
    pub effect: Option<Vec<String>>,
}

/// Structure-pattern field (`fpat`).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldPat {
    Field(String, Pat),
    Wild,
}

/// Sail patterns (`pat` / `atomic_pat`).
#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Wild,
    Lit(Lit),
    Id(String),
    KidVar(String),
    App(String, Vec<Pat>),
    VectorSubrange(String, String, String),
    Tuple(Vec<Pat>),
    Vector(Vec<Pat>),
    List(Vec<Pat>),
    Struct(Option<String>, Vec<FieldPat>),
    Typ(Box<Pat>, Typ),
    Infix(Box<Pat>, Vec<(PatOp, Pat)>),
    As(Box<Pat>, Typ),
    Attr(Vec<Attr>, Box<Pat>),
}

/// Pattern infix operator (`pat_op`): `@`, `::`, `^`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatOp {
    VectorConcat,
    Cons,
    StringAppend,
}

/// `foreach` loop header (`loop_exp`).
#[derive(Debug, Clone, PartialEq)]
pub struct LoopHead {
    pub var: String,
    pub from: Box<Exp>,
    pub ord: LoopOrd,
    pub to: Box<Exp>,
    pub step: Option<Box<Exp>>,
    pub order: Option<Typ>,
}

/// `to` / `downto` direction marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOrd {
    To,
    Downto,
}

/// Loop kind for `while` / `repeat` / `until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopKind {
    While,
    Until,
}

/// Termination measure annotation.
#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    None,
    Some(Box<Exp>),
}

/// Match/function clause body (`case` / `funcl_patexp`).
#[derive(Debug, Clone, PartialEq)]
pub enum PatExp {
    Exp(Pat, Box<Exp>),
    When(Pat, Box<Exp>, Box<Exp>),
    Attr(Vec<Attr>, Box<PatExp>),
}

/// Structure-expression field (`fexp_exp`).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldExp {
    Field(Box<Exp>, Box<Exp>),
    Punning(String),
}

/// Vector update (`vector_update`).
#[derive(Debug, Clone, PartialEq)]
pub enum VectorUpdate {
    Single(Box<Exp>, Box<Exp>),
    Range(Box<Exp>, Box<Exp>, Box<Exp>),
}

/// Trailing part of a vector update after the head expression.
#[derive(Debug, Clone, PartialEq)]
pub enum VecUpdTail {
    Eq(Exp),
    Range(Exp, Exp),
}

/// Suffix of an atomic expression after the head (field access, type
/// annotation, vector indexing/slicing). Applied left to right.
#[derive(Debug, Clone, PartialEq)]
pub enum ExpSuffix {
    Field(String),
    Call(String, Vec<Exp>),
    TypAnnot(Typ),
    Index(Box<Exp>),
    Subrange(Box<Exp>, Box<Exp>),
    Slice(Box<Exp>, Box<Exp>),
}

/// Sail expressions (`exp` / `atomic_exp`).
#[derive(Debug, Clone, PartialEq)]
pub enum Exp {
    Id(String),
    Ref(String),
    Lit(Lit),
    Undef,
    Config(String),
    Exit(Box<Exp>),
    Sizeof(Typ),
    Constraint(Typ),
    Assert(Box<Exp>, Box<Exp>),
    App(String, Vec<Exp>),
    Suffix(Box<Exp>, Vec<ExpSuffix>),
    Assign(Box<Exp>, Box<Exp>),
    Let(Pat, Box<Exp>, Box<Exp>),
    Var(Box<Exp>, Box<Exp>, Box<Exp>),
    Block(Vec<Exp>),
    Return(Box<Exp>),
    Throw(Box<Exp>),
    If(Box<Exp>, Box<Exp>, Box<Exp>),
    Match(Box<Exp>, Vec<PatExp>),
    Try(Box<Exp>, Vec<PatExp>),
    For(Box<LoopHead>, Box<Exp>),
    Loop(LoopKind, Measure, Box<Exp>, Box<Exp>),
    Infix(Box<InfixChain<Exp>>),
    Struct(Option<String>, Vec<FieldExp>),
    StructUpdate(Box<Exp>, Vec<FieldExp>),
    Vector(Vec<Exp>),
    VectorUpdate(Box<Exp>, Vec<VectorUpdate>),
    List(Vec<Exp>),
    Tuple(Vec<Exp>),
    InternalPlet(Pat, Box<Exp>, Box<Exp>),
    InternalReturn(Box<Exp>),
    InternalAssume(Typ, Box<Exp>),
    Attr(Vec<Attr>, Box<Exp>),
}

/// Attribute entry: `$[name data?, ...]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub data: Option<AttrData>,
}

/// Attribute payload (`attribute_data`).
#[derive(Debug, Clone, PartialEq)]
pub enum AttrData {
    Object(Vec<(String, AttrData)>),
    Num(String),
    Str(String),
    Bool(bool),
    List(Vec<AttrData>),
}

/// Documentation comment (`doc_comment`).
#[derive(Debug, Clone, PartialEq)]
pub enum DocComment {
    Block(String),
    Line(String),
}

/// `private` / attribute / doc prefix on function clauses and definitions.
#[derive(Debug, Clone, PartialEq)]
pub enum Annot {
    Private,
    Attr(Vec<Attr>),
    Doc(DocComment),
}

/// Guarded clause pattern: plain `pat` or `(pat if guard)`.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardedPat {
    Plain(Pat),
    Guarded(Pat, Exp),
}

/// Single function clause (`funcl`).
#[derive(Debug, Clone, PartialEq)]
pub struct Funcl {
    pub annots: Vec<Annot>,
    pub name: String,
    pub pat: GuardedPat,
    pub typ: Option<(Option<TypQuant>, Typ)>,
    pub body: PatExpBody,
}

/// Right-hand side of a function clause.
#[derive(Debug, Clone, PartialEq)]
pub enum PatExpBody {
    Exp(Box<Exp>),
    When(Box<Exp>, Box<Exp>),
}

/// Function definition (`fun_def`): optional recursion measure, optional
/// result-type annotation, `and`-chained clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct FunDef {
    pub rec_measure: Option<(Pat, Exp)>,
    pub head: Funcl,
    pub tail: Vec<Funcl>,
}

/// Tail after a pattern head identifier (`atomic_pat`).
#[derive(Debug, Clone, PartialEq)]
pub enum PatIdTail {
    Call(Vec<Pat>),
    Subrange(String, String),
}

/// Tail after a mapping-pattern head identifier (`atomic_mpat`).
#[derive(Debug, Clone, PartialEq)]
pub enum MPatIdTail {
    Call(Vec<MPat>),
    Subrange(String, String),
}

/// Suffix of a mapping pattern: operator chain or `as` binding.
#[derive(Debug, Clone, PartialEq)]
pub enum MPatSuffix {
    Chain(Vec<(PatOp, MPat)>),
    As(String),
}

/// Mapping pattern (`mpat` / `atomic_mpat`).
#[derive(Debug, Clone, PartialEq)]
pub enum MPat {
    Lit(Lit),
    Id(String),
    App(String, Vec<MPat>),
    VectorSubrange(String, String, String),
    Tuple(Vec<MPat>),
    Vector(Vec<MPat>),
    List(Vec<MPat>),
    Struct(Option<String>, Vec<(String, MPat)>),
    Typ(Box<MPat>, Typ),
    Infix(Box<MPat>, Vec<(PatOp, MPat)>),
    As(Box<MPat>, String),
}

/// Mapping-pattern expression (`mpexp`): pattern with optional `if` guard.
#[derive(Debug, Clone, PartialEq)]
pub struct MPExp {
    pub pat: MPat,
    pub guard: Option<Exp>,
}

/// Mapping clause (`mapcl`).
#[derive(Debug, Clone, PartialEq)]
pub enum MapCl {
    Bidir(MPExp, MPExp),
    Forwards(PatExp),
    ForwardsDeprecated(MPExp, Exp),
    Backwards(PatExp),
    When(Box<MapCl>, Exp),
    Attr(Vec<Attr>, Box<MapCl>),
    Doc(DocComment, Box<MapCl>),
}

/// Mapping definition (`map_def`).
#[derive(Debug, Clone, PartialEq)]
pub struct MapDef {
    pub name: String,
    pub typschm: Option<TypSchm>,
    pub clauses: Vec<MapCl>,
}

/// Index range for bitfields (`index_range`).
#[derive(Debug, Clone, PartialEq)]
pub enum IndexRange {
    Single(Typ),
    Range(Typ, Typ),
    Concat(Box<IndexRange>, Box<IndexRange>),
}

/// Register field annotation item (`r_id_def`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegField {
    Item(String, IndexRange),
    Doc(DocComment, Box<RegField>),
    Attr(Vec<Attr>, Box<RegField>),
}

/// Type definition (`type_def`).
#[derive(Debug, Clone, PartialEq)]
pub enum TypeDef {
    Abbrev(String, TypQuant, Option<Kind>, Typ),
    Abstract(String, Kind, Option<Vec<String>>),
    Record(String, TypQuant, Vec<StructField>),
    Enum(String, Vec<(String, Typ)>, Vec<(EnumMember, Option<Exp>)>),
    Variant(String, TypQuant, Vec<TypeUnion>, bool),
    Bitfield(String, Typ, Vec<RegField>),
}

/// Record field (`struct_field`).
#[derive(Debug, Clone, PartialEq)]
pub enum StructField {
    Item(String, Typ),
    Doc(DocComment, Box<StructField>),
    Attr(Vec<Attr>, Box<StructField>),
}

/// Enum member (`enum_member`).
#[derive(Debug, Clone, PartialEq)]
pub enum EnumMember {
    Item(String),
    Doc(DocComment, Box<EnumMember>),
    Attr(Vec<Attr>, Box<EnumMember>),
}

/// Union constructor (`type_union`).
#[derive(Debug, Clone, PartialEq)]
pub enum TypeUnion {
    Id(String, Typ),
    AnonRecord(String, Vec<StructField>),
    Private(Box<TypeUnion>),
    Attr(Vec<Attr>, Box<TypeUnion>),
    Doc(DocComment, Box<TypeUnion>),
}

/// Value specification (`val_spec_def`).
#[derive(Debug, Clone, PartialEq)]
pub struct ValSpec {
    pub name: String,
    pub typschm: TypSchm,
    pub ext: Option<Extern>,
}

/// `extern` block of a `val` spec.
#[derive(Debug, Clone, PartialEq)]
pub struct Extern {
    pub pure: Option<bool>,
    pub bindings: Vec<(String, String)>,
}

/// Register definition (`register_def`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegDef {
    pub name: String,
    pub typ: Typ,
    pub init: Option<Exp>,
}

/// Default annotation (`default_def`).
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultDef {
    pub kind: Kind,
    pub order: Typ,
}

/// Scattered definition (`scattered_def`).
#[derive(Debug, Clone, PartialEq)]
pub enum ScatteredDef {
    Enum(String),
    Variant(String, TypQuant),
    Function(String),
    Mapping(String, Option<Typ>),
    EnumClause(String, String),
    FunClause(Funcl),
    UnionClause(String, TypeUnion),
    MapClause(String, MapCl),
    End(String),
}

/// Instantiation substitution (`subst`).
#[derive(Debug, Clone, PartialEq)]
pub enum Subst {
    Typ(String, Typ),
    Id(String, String),
}

/// Pragma value (`def_aux` pragma forms).
#[derive(Debug, Clone, PartialEq)]
pub enum Pragma {
    Line(String, String),
    Structured(String, Vec<(String, AttrData)>),
}

/// Top-level definition (`def_aux`, without doc/attribute/private wrappers).
#[derive(Debug, Clone, PartialEq)]
pub enum DefAux {
    Fun(FunDef),
    Map(MapDef),
    Impl(Funcl),
    Fixity(FixAssoc, u8, String),
    Val(ValSpec),
    Outcome(OutcomeSpec, Vec<Def>),
    Instantiation(String, Vec<Subst>),
    Type(TypeDef),
    Let(Pat, Exp),
    Register(RegDef),
    Overload(String, Vec<String>),
    Default(DefaultDef),
    Constraint(Typ),
    MutRec(Vec<FunDef>),
    Pragma(Pragma),
    Measure(String, Pat, Exp),
    LoopMeasures(String, Vec<(LoopKind, Exp)>),
    Scattered(ScatteredDef),
}

/// `val ... : ...` outcome header (`outcome_spec_def`).
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeSpec {
    pub name: String,
    pub typschm: TypSchm,
    pub param: Option<TypQuant>,
}

/// Fixity associativity (`Fixity` token).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixAssoc {
    Infix,
    InfixL,
    InfixR,
}

/// Top-level definition with optional wrappers (`def`).
#[derive(Debug, Clone, PartialEq)]
pub enum Def {
    Aux(DefAux),
    Private(Box<Def>),
    Attr(Vec<Attr>, Box<Def>),
    Doc(DocComment, Box<Def>),
}
