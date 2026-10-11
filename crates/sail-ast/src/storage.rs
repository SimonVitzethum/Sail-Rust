#![forbid(unsafe_code)]
//! Depth-independent storage of a definition and its worker-local symbols.
//!
//! Recursive children are moved into flat, typed records before serde sees
//! them. Reading, reconstruction and disposal use the same exhaustive child
//! traversal. The canonical AST and its public JSON format stay unchanged.

use crate::*;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

const FORMAT: &str = "sail-rs/flat-definition-v1";

trait Children {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>));
}
trait Empty {
    fn empty() -> Self;
}

macro_rules! parts {
    ($($variant:ident($ty:ty)),+ $(,)?) => {
        #[derive(Serialize, Deserialize)]
        enum Part { $($variant($ty)),+ }
        enum Slot<'a> { $($variant(&'a mut $ty)),+ }
        impl Part {
            fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
                match self { $(Self::$variant(node) => node.children(visit)),+ }
            }
        }
        impl Slot<'_> {
            fn take(self) -> Part {
                match self { $(Self::$variant(node) =>
                    Part::$variant(std::mem::replace(node, <$ty>::empty()))),+ }
            }
            fn is_placeholder(&self) -> bool {
                match self { $(Self::$variant(node) => **node == <$ty>::empty()),+ }
            }
            fn accepts(&self, tag: std::mem::Discriminant<Part>) -> bool {
                let empty = match self { $(Self::$variant(_) => Part::$variant(<$ty>::empty())),+ };
                std::mem::discriminant(&empty) == tag
            }
            fn put(self, part: Part) {
                match (self, part) { $((Self::$variant(slot), Part::$variant(node)) => *slot = node),+,
                    _ => unreachable!("validated flat AST child kind"),
                }
            }
        }
    };
}
parts! {
    Def(Def), TypeDef(TypeDef), Typ(Typ), TypArg(TypArg), NExp(NExp),
    Constraint(Constraint), Pat(Pat), TypPat(TypPat), Exp(Exp), LExp(LExp),
    IndexRange(IndexRange), MPExp(MPExp), Lit(Lit), Value(Value), Data(AttributeData),
}

impl<K: Empty> Empty for Node<K> {
    fn empty() -> Self {
        Node::dummy(K::empty())
    }
}
macro_rules! empty {
    ($($ty:ty => $value:expr),+ $(,)?) => { $(impl Empty for $ty {
        fn empty() -> Self { $value }
    })+ };
}
empty! {
    DefKind => DefKind::Pragma { key: String::new(), payload: String::new() },
    TypeDefKind => TypeDefKind::Enum { name: Name::dummy(Symbol::DUMMY), members: vec![], wf: false },
    TypKind => TypKind::Unknown,
    TypArgKind => TypArgKind::Typ(Typ::empty()),
    NExpKind => NExpKind::Constant(String::new()),
    ConstraintKind => ConstraintKind::True,
    PatKind => PatKind::Wild,
    TypPatKind => TypPatKind::Wild,
    ExpKind => ExpKind::Undef,
    LExpKind => LExpKind::Tuple(vec![]),
    IndexRangeKind => IndexRangeKind::Single(NExp::empty()),
    MPExpKind => MPExpKind::Pat(Pat::empty()),
    LitKind => LitKind::Unit,
    Value => Value::Unit,
    AttributeData => AttributeData::Bool(false),
}

impl<K: Children> Children for Node<K> {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        if let Some(annotation) = &mut self.annotation {
            for attribute in &mut annotation.attributes {
                if let Some(data) = &mut attribute.data {
                    visit(Slot::Data(data));
                }
            }
        }
        self.kind.children(visit);
    }
}
impl Children for LitKind {
    fn children(&mut self, _: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Unit
            | Self::True
            | Self::False
            | Self::Num(_)
            | Self::Hex(_)
            | Self::Bin(_)
            | Self::String(_)
            | Self::Real(_) => {}
        }
    }
}
impl Children for NExpKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Id(_) | Self::Var(_) | Self::Constant(_) => {}
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::NExp(arg));
                }
            }
            Self::If {
                cond,
                then_branch,
                else_branch,
            } => {
                visit(Slot::Constraint(cond));
                visit(Slot::NExp(then_branch));
                visit(Slot::NExp(else_branch));
            }
            Self::Times(a, b) | Self::Sum(a, b) | Self::Minus(a, b) => {
                visit(Slot::NExp(a));
                visit(Slot::NExp(b));
            }
            Self::Exp(a) | Self::Neg(a) => visit(Slot::NExp(a)),
        }
    }
}
impl Children for TypArgKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::NExp(n) => visit(Slot::NExp(n)),
            Self::Typ(t) => visit(Slot::Typ(t)),
            Self::Bool(c) => visit(Slot::Constraint(c)),
        }
    }
}
impl Children for TypKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Unknown | Self::Id(_) | Self::Var(_) => {}
            Self::Fn { args, ret } => {
                for arg in args {
                    visit(Slot::Typ(arg));
                }
                visit(Slot::Typ(ret));
            }
            Self::Bidir { lhs, rhs } => {
                visit(Slot::Typ(lhs));
                visit(Slot::Typ(rhs));
            }
            Self::Tuple(items) => {
                for item in items {
                    visit(Slot::Typ(item));
                }
            }
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::TypArg(arg));
                }
            }
            Self::Exist {
                constraint, typ, ..
            } => {
                visit(Slot::Constraint(constraint));
                visit(Slot::Typ(typ));
            }
        }
    }
}
impl Children for ConstraintKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Equal(a, b) | Self::NotEqual(a, b) => {
                visit(Slot::TypArg(a));
                visit(Slot::TypArg(b));
            }
            Self::Ge(a, b) | Self::Gt(a, b) | Self::Le(a, b) | Self::Lt(a, b) => {
                visit(Slot::NExp(a));
                visit(Slot::NExp(b));
            }
            Self::Set { nexp, .. } => visit(Slot::NExp(nexp)),
            Self::And(a, b) | Self::Or(a, b) => {
                visit(Slot::Constraint(a));
                visit(Slot::Constraint(b));
            }
            Self::Not(a) => visit(Slot::Constraint(a)),
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::TypArg(arg));
                }
            }
            Self::Id(_) | Self::Var(_) | Self::True | Self::False => {}
        }
    }
}
impl Children for TypPatKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Wild | Self::Var(_) => {}
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::TypPat(arg));
                }
            }
        }
    }
}
impl Children for PatKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Lit(lit) => visit(Slot::Lit(lit)),
            Self::Wild | Self::Id(_) | Self::VectorSubrange { .. } => {}
            Self::Or(a, b) | Self::Cons(a, b) => {
                visit(Slot::Pat(a));
                visit(Slot::Pat(b));
            }
            Self::Not(pat) | Self::As { pat, .. } => visit(Slot::Pat(pat)),
            Self::Typ { typ, pat } => {
                visit(Slot::Typ(typ));
                visit(Slot::Pat(pat));
            }
            Self::Var { pat, typ_pat } => {
                visit(Slot::Pat(pat));
                visit(Slot::TypPat(typ_pat));
            }
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::Pat(arg));
                }
            }
            Self::Vector(items)
            | Self::VectorConcat(items)
            | Self::Tuple(items)
            | Self::List(items)
            | Self::StringAppend(items) => {
                for item in items {
                    visit(Slot::Pat(item));
                }
            }
            Self::Struct { fields, .. } => {
                for (_, pat) in fields {
                    visit(Slot::Pat(pat));
                }
            }
        }
    }
}
impl Children for LExpKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Id(_) => {}
            Self::Deref(exp) => visit(Slot::Exp(exp)),
            Self::Typ { typ, .. } => visit(Slot::Typ(typ)),
            Self::Tuple(items) | Self::VectorConcat(items) => {
                for item in items {
                    visit(Slot::LExp(item));
                }
            }
            Self::Vector { lexp, index } => {
                visit(Slot::LExp(lexp));
                visit(Slot::Exp(index));
            }
            Self::VectorRange { lexp, lo, hi } => {
                visit(Slot::LExp(lexp));
                visit(Slot::Exp(lo));
                visit(Slot::Exp(hi));
            }
            Self::Field { lexp, .. } => visit(Slot::LExp(lexp)),
        }
    }
}
impl Children for PExpKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::PatExp { pat, body } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(body));
            }
            Self::PatWhen { pat, guard, body } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(guard));
                visit(Slot::Exp(body));
            }
        }
    }
}
impl Children for ExpKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Lit(lit) => visit(Slot::Lit(lit)),
            Self::Id(_) | Self::Config(_) | Self::Ref(_) | Self::Undef => {}
            Self::Block(items) | Self::Tuple(items) | Self::Vector(items) | Self::List(items) => {
                for item in items {
                    visit(Slot::Exp(item));
                }
            }
            Self::App { args, .. } => {
                for arg in args {
                    visit(Slot::Exp(arg));
                }
            }
            Self::Typ { typ, exp } => {
                visit(Slot::Typ(typ));
                visit(Slot::Exp(exp));
            }
            Self::If {
                cond,
                then_branch,
                else_branch,
            } => {
                visit(Slot::Exp(cond));
                visit(Slot::Exp(then_branch));
                visit(Slot::Exp(else_branch));
            }
            Self::Loop {
                measure,
                cond,
                body,
                ..
            } => {
                if let Some(measure) = measure {
                    visit(Slot::Exp(measure));
                }
                visit(Slot::Exp(cond));
                visit(Slot::Exp(body));
            }
            Self::For {
                from,
                to,
                step,
                body,
                ..
            } => {
                visit(Slot::Exp(from));
                visit(Slot::Exp(to));
                visit(Slot::Exp(step));
                visit(Slot::Exp(body));
            }
            Self::VectorAppend(a, b) | Self::Cons(a, b) => {
                visit(Slot::Exp(a));
                visit(Slot::Exp(b));
            }
            Self::Struct { fields, .. } => {
                for field in fields {
                    visit(Slot::Exp(&mut field.value));
                }
            }
            Self::StructUpdate { record, fields } => {
                visit(Slot::Exp(record));
                for field in fields {
                    visit(Slot::Exp(&mut field.value));
                }
            }
            Self::Field { record, .. } => visit(Slot::Exp(record)),
            Self::Match { scrutinee, arms } => {
                visit(Slot::Exp(scrutinee));
                for arm in arms {
                    arm.kind.children(visit);
                }
            }
            Self::Let { pat, bind, body } | Self::InternalPlet { pat, bind, body } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(bind));
                visit(Slot::Exp(body));
            }
            Self::Assign { lexp, value } => {
                visit(Slot::LExp(lexp));
                visit(Slot::Exp(value));
            }
            Self::Sizeof(nexp) => visit(Slot::NExp(nexp)),
            Self::Return(exp) | Self::Exit(exp) | Self::Throw(exp) | Self::InternalReturn(exp) => {
                visit(Slot::Exp(exp))
            }
            Self::Try { exp, arms } => {
                visit(Slot::Exp(exp));
                for arm in arms {
                    arm.kind.children(visit);
                }
            }
            Self::Assert { cond, msg } => {
                visit(Slot::Exp(cond));
                visit(Slot::Exp(msg));
            }
            Self::Var { lexp, init, body } => {
                visit(Slot::LExp(lexp));
                visit(Slot::Exp(init));
                visit(Slot::Exp(body));
            }
            Self::InternalValue(value) => visit(Slot::Value(value)),
            Self::InternalAssume { constraint, exp } => {
                visit(Slot::Constraint(constraint));
                visit(Slot::Exp(exp));
            }
            Self::Constraint(constraint) => visit(Slot::Constraint(constraint)),
        }
    }
}
impl Children for Value {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Tuple(items) | Self::List(items) => {
                for item in items {
                    visit(Slot::Value(item));
                }
            }
            Self::Unit | Self::Bool(_) | Self::Int(_) | Self::String(_) => {}
        }
    }
}
impl Children for AttributeData {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Object(fields) => {
                for (_, value) in fields {
                    visit(Slot::Data(value));
                }
            }
            Self::List(items) => {
                for item in items {
                    visit(Slot::Data(item));
                }
            }
            Self::Num(_) | Self::Str(_) | Self::Bool(_) => {}
        }
    }
}
impl Children for IndexRangeKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Single(nexp) => visit(Slot::NExp(nexp)),
            Self::Range { lo, hi } => {
                visit(Slot::NExp(lo));
                visit(Slot::NExp(hi));
            }
            Self::Concat(a, b) => {
                visit(Slot::IndexRange(a));
                visit(Slot::IndexRange(b));
            }
        }
    }
}
impl Children for MPExpKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Pat(pat) => visit(Slot::Pat(pat)),
            Self::When { pat, guard } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(guard));
            }
        }
    }
}
impl Children for MapclKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Bidir {
                forwards,
                backwards,
            } => {
                visit(Slot::MPExp(forwards));
                visit(Slot::MPExp(backwards));
            }
            Self::Forwards(arm) | Self::Backwards(arm) => arm.kind.children(visit),
        }
    }
}
impl Children for Vec<QuantItem> {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        for item in self {
            match item {
                QuantItem::Id(_) => {}
                QuantItem::Constraint(c) => visit(Slot::Constraint(c)),
            }
        }
    }
}
impl Children for TypSchm {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        self.quant.children(visit);
        visit(Slot::Typ(&mut self.typ));
    }
}
impl Children for TAnnotOpt {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::None => {}
            Self::Some { quant, typ } => {
                quant.children(visit);
                visit(Slot::Typ(typ));
            }
        }
    }
}
impl Children for RecOpt {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::NonRec | Self::Rec => {}
            Self::Measure { pat, exp } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(exp));
            }
        }
    }
}
impl Children for TypeDefKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Abbrev { quant, arg, .. } => {
                quant.children(visit);
                visit(Slot::TypArg(arg));
            }
            Self::Record { quant, fields, .. } => {
                quant.children(visit);
                for (_, typ) in fields {
                    visit(Slot::Typ(typ));
                }
            }
            Self::Variant { quant, ctors, .. } => {
                quant.children(visit);
                for ctor in ctors {
                    visit(Slot::Typ(&mut ctor.typ));
                }
            }
            Self::Bitfield { typ, fields, .. } => {
                visit(Slot::Typ(typ));
                for (_, range) in fields {
                    visit(Slot::IndexRange(range));
                }
            }
            Self::Enum { .. } | Self::Abstract { .. } => {}
        }
    }
}
impl Children for FunclSet {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        self.rec.children(visit);
        self.annot.children(visit);
        for funcl in &mut self.funcls {
            funcl.arm.kind.children(visit);
        }
    }
}
impl Children for ScatteredDef {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Function { annot, .. } | Self::Mapping { annot, .. } => annot.children(visit),
            Self::Funcl(funcl) => funcl.arm.kind.children(visit),
            Self::Variant { quant, .. } => quant.children(visit),
            Self::Unioncl { union, .. } => visit(Slot::Typ(&mut union.typ)),
            Self::Mapcl { clause, .. } => clause.kind.children(visit),
            Self::Enum(_) | Self::Enumcl { .. } | Self::End(_) => {}
        }
    }
}
impl Children for DefKind {
    fn children(&mut self, visit: &mut impl FnMut(Slot<'_>)) {
        match self {
            Self::Type(typ) => visit(Slot::TypeDef(typ)),
            Self::Constraint(c) => visit(Slot::Constraint(c)),
            Self::Fundef { rec, annot, funcls } => {
                rec.children(visit);
                annot.children(visit);
                for funcl in funcls {
                    funcl.arm.kind.children(visit);
                }
            }
            Self::Mapdef { annot, clauses, .. } => {
                annot.children(visit);
                for clause in clauses {
                    clause.kind.children(visit);
                }
            }
            Self::Impl(funcl) => funcl.arm.kind.children(visit),
            Self::Let { pat, exp } | Self::Measure { pat, exp, .. } => {
                visit(Slot::Pat(pat));
                visit(Slot::Exp(exp));
            }
            Self::Val(val) => val.typschm.children(visit),
            Self::Outcome {
                typschm,
                quant,
                defs,
                ..
            } => {
                typschm.children(visit);
                quant.children(visit);
                for def in defs {
                    visit(Slot::Def(def));
                }
            }
            Self::Instantiation { substs, .. } => {
                for subst in substs {
                    match subst {
                        Subst::Typ { arg, .. } => visit(Slot::TypArg(arg)),
                        Subst::Id { .. } => {}
                    }
                }
            }
            Self::Scattered(def) => def.children(visit),
            Self::LoopMeasures { measures, .. } => {
                for (_, exp) in measures {
                    visit(Slot::Exp(exp));
                }
            }
            Self::Register(reg) => {
                visit(Slot::Typ(&mut reg.typ));
                if let Some(init) = &mut reg.init {
                    visit(Slot::Exp(init));
                }
            }
            Self::InternalMutrec(groups) => {
                for group in groups {
                    group.children(visit);
                }
            }
            Self::Fixity { .. }
            | Self::Overload { .. }
            | Self::Default(_)
            | Self::Pragma { .. } => {}
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    node: Option<Part>,
    children: Vec<usize>,
}

/// An owned, flat representation of one canonical definition.
/// Its destructor also handles partially reconstructed trees without recursion.
pub struct DefinitionSnapshot {
    records: Vec<Record>,
}

impl DefinitionSnapshot {
    /// Move children into flat records, preserving all node IDs and annotations.
    #[must_use]
    pub fn new(def: Def) -> Self {
        let mut records = vec![Record {
            node: Some(Part::Def(def)),
            children: vec![],
        }];
        let mut index = 0;
        while index < records.len() {
            let base = records.len();
            let mut pending = Vec::new();
            let record = &mut records[index];
            record.node.as_mut().unwrap().children(&mut |slot| {
                record.children.push(base + pending.len());
                pending.push(Record {
                    node: Some(slot.take()),
                    children: vec![],
                });
            });
            records.extend(pending);
            index += 1;
        }
        Self { records }
    }

    /// Write bounded-depth JSON records. Symbol order, unlike hash-table
    /// iteration order, is stable across workers and invocations.
    pub fn write(&self, mut writer: impl Write, interner: &SimpleInterner) -> io::Result<()> {
        #[derive(Serialize)]
        struct Header<'a> {
            format: &'a str,
            nodes: usize,
            symbols: Vec<&'a str>,
        }
        let symbols = (0..interner.len())
            .map(|i| interner.resolve(Symbol(i as u32)).unwrap())
            .collect();
        let header = Header {
            format: FORMAT,
            nodes: self.records.len(),
            symbols,
        };
        serde_json::to_writer(&mut writer, &header)?;
        writer.write_all(b"\n")?;
        for (index, record) in self.records.iter().enumerate().rev() {
            serde_json::to_writer(&mut writer, &(index, record))?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Read and validate a complete definition, without recursive AST decoding.
    /// Children precede their parents; a malformed reference never reaches
    /// reconstruction. The reader does not allocate from an untrusted count.
    pub fn read(mut reader: impl BufRead) -> io::Result<(Self, SimpleInterner)> {
        #[derive(Deserialize)]
        struct Header {
            format: String,
            nodes: usize,
            symbols: Vec<String>,
        }
        let mut line = String::new();
        read_line(&mut reader, &mut line)?;
        let header: Header = serde_json::from_str(&line)?;
        if header.format != FORMAT || header.nodes == 0 {
            return Err(invalid("flat AST format or empty root"));
        }
        let mut interner = SimpleInterner::new();
        for (index, symbol) in header.symbols.into_iter().enumerate() {
            if usize::try_from(interner.intern(&symbol).0).ok() != Some(index) {
                return Err(invalid("flat AST duplicate or overflowing symbol"));
            }
        }
        let mut records = Vec::new();
        for expected in (0..header.nodes).rev() {
            read_line(&mut reader, &mut line)?;
            let (index, record): (usize, Record) = serde_json::from_str(&line)?;
            if index != expected {
                return Err(invalid("flat AST record order"));
            }
            records.push(record);
        }
        if !reader.fill_buf()?.is_empty() {
            return Err(invalid("flat AST trailing data"));
        }
        records.reverse();
        let mut snapshot = Self { records };
        snapshot.validate()?;
        Ok((snapshot, interner))
    }

    fn validate(&mut self) -> io::Result<()> {
        let tags = self
            .records
            .iter()
            .map(|r| {
                r.node
                    .as_ref()
                    .map(std::mem::discriminant)
                    .ok_or_else(|| invalid("flat AST missing node"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        if !matches!(self.records[0].node, Some(Part::Def(_))) {
            return Err(invalid("flat AST root must be a definition"));
        }
        let mut seen = vec![false; self.records.len()];
        seen[0] = true;
        for (index, record) in self.records.iter_mut().enumerate() {
            let mut children = record.children.iter();
            let mut valid = true;
            record.node.as_mut().unwrap().children(&mut |slot| {
                let Some(&child) = children.next() else {
                    valid = false;
                    return;
                };
                if child <= index
                    || child >= tags.len()
                    || seen[child]
                    || !slot.accepts(tags[child])
                    || !slot.is_placeholder()
                {
                    valid = false;
                    return;
                }
                seen[child] = true;
            });
            if !valid || children.next().is_some() {
                return Err(invalid("flat AST invalid child reference or placeholder"));
            }
        }
        if seen.iter().any(|seen| !seen) {
            return Err(invalid("flat AST orphan node"));
        }
        Ok(())
    }

    /// Reassemble the canonical AST by moving each child exactly once.
    #[must_use]
    pub fn into_definition(mut self) -> Def {
        for index in (0..self.records.len()).rev() {
            let mut node = self.records[index].node.take().unwrap();
            let children = std::mem::take(&mut self.records[index].children);
            let mut children = children.into_iter();
            node.children(&mut |slot| {
                let child = children.next().unwrap();
                slot.put(self.records[child].node.take().unwrap());
            });
            self.records[index].node = Some(node);
        }
        let Some(Part::Def(def)) = self.records[0].node.take() else {
            unreachable!("validated definition root")
        };
        def
    }
}
impl Drop for DefinitionSnapshot {
    fn drop(&mut self) {
        for record in &mut self.records {
            if let Some(node) = record.node.take() {
                drop_part(node);
            }
        }
    }
}

/// Release a canonical definition using an explicit work stack, including
/// recursive type, pattern, expression, value and annotation children.
pub fn drop_definition(def: Def) {
    drop_part(Part::Def(def));
}

fn drop_part(part: Part) {
    let mut pending = vec![part];
    while let Some(mut part) = pending.pop() {
        part.children(&mut |slot| pending.push(slot.take()));
        // Only fixed-size empty placeholders remain in this node.
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn read_line(reader: &mut impl BufRead, line: &mut String) -> io::Result<()> {
    line.clear();
    if reader.read_line(line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "flat AST truncated",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn let_def(pat: Pat, exp: Exp) -> Def {
        Node::dummy(DefKind::Let { pat, exp })
    }
    fn bytes(snapshot: &DefinitionSnapshot, it: &SimpleInterner) -> Vec<u8> {
        let mut bytes = Vec::new();
        snapshot.write(&mut bytes, it).unwrap();
        bytes
    }

    #[test]
    fn deep_trees_roundtrip_and_drop_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut pat = Pat::empty();
                let mut exp = Exp::empty();
                let mut typ = Typ::empty();
                let mut nexp = NExp::empty();
                let mut data = AttributeData::Str("retained".into());
                let mut value = Value::Int("42".into());
                for _ in 0..10_000 {
                    pat = Node::dummy(PatKind::Not(Box::new(pat)));
                    exp = Node::dummy(ExpKind::Return(Box::new(exp)));
                    typ = Node::dummy(TypKind::Tuple(vec![typ]));
                    nexp = Node::dummy(NExpKind::Neg(Box::new(nexp)));
                    data = AttributeData::List(vec![data]);
                    value = Value::Tuple(vec![value]);
                }
                let mut root = let_def(
                    pat,
                    Node::dummy(ExpKind::Tuple(vec![
                        exp,
                        Node::dummy(ExpKind::Typ {
                            typ,
                            exp: Box::new(Exp::empty()),
                        }),
                        Node::dummy(ExpKind::Sizeof(nexp)),
                        Node::dummy(ExpKind::InternalValue(value)),
                    ])),
                );
                root.id = NodeId::new(123, 4);
                root.annotation_mut().attributes.push(Attribute {
                    name: "nested".into(),
                    data: Some(data),
                });
                let it = SimpleInterner::new();
                let snapshot = DefinitionSnapshot::new(root);
                let encoded = bytes(&snapshot, &it);
                drop(snapshot);
                let (snapshot, back_it) = DefinitionSnapshot::read(encoded.as_slice()).unwrap();
                assert_eq!(bytes(&snapshot, &back_it), encoded);
                let root = snapshot.into_definition();
                assert_eq!(root.id, NodeId::new(123, 4));
                // Flatten once more to check that reconstruction retained every
                // child, without recursive Eq/Clone/serialization of the tree.
                let snapshot = DefinitionSnapshot::new(root);
                assert_eq!(bytes(&snapshot, &back_it), encoded);
                drop_definition(snapshot.into_definition());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn symbols_metadata_and_payloads_are_preserved_deterministically() {
        let mut it = SimpleInterner::new();
        let a = it.intern("alpha");
        let b = it.intern("beta");
        let mut def = let_def(
            Node::dummy(PatKind::Id(Name::dummy(a))),
            Node::dummy(ExpKind::Lit(Node::dummy(LitKind::String(b)))),
        );
        def.span = Span {
            file: FileId(7),
            start: 12,
            end: 91,
        };
        def.annotation_mut().documentation = Some("source documentation".into());
        def.annotation_mut().private = true;
        let expected = def.clone();
        let snapshot = DefinitionSnapshot::new(def);
        let wire = bytes(&snapshot, &it);
        let (back, back_it) = DefinitionSnapshot::read(wire.as_slice()).unwrap();
        assert_eq!(back.into_definition(), expected);
        assert_eq!(it, back_it);
        let json = serde_json::to_string(&it).unwrap();
        let reconstructed: SimpleInterner = serde_json::from_str(&json).unwrap();
        assert_eq!(bytes(&snapshot, &reconstructed), wire);
    }

    #[test]
    fn malformed_graphs_are_rejected_before_reconstruction() {
        let source = DefinitionSnapshot::new(let_def(Pat::empty(), Exp::empty()));
        let wire = bytes(&source, &SimpleInterner::new());
        let original: Vec<serde_json::Value> = std::str::from_utf8(&wire)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let bad = |lines: Vec<serde_json::Value>| {
            let mut bytes = Vec::new();
            for line in lines {
                serde_json::to_writer(&mut bytes, &line).unwrap();
                bytes.push(b'\n');
            }
            assert!(DefinitionSnapshot::read(bytes.as_slice()).is_err());
        };
        // Root is last: its child slots are pattern and expression.
        for children in [
            vec![0, 2],
            vec![1, 1],
            vec![2, 1],
            vec![1, 99],
            vec![1],
            vec![1, 2, 2],
        ] {
            let mut lines = original.clone();
            lines.last_mut().unwrap()[1]["children"] = serde_json::json!(children);
            bad(lines);
        }
        let mut lines = original.clone();
        lines[0]["format"] = "unknown".into();
        bad(lines);
        let mut lines = original.clone();
        lines[0]["symbols"] = serde_json::json!(["duplicate", "duplicate"]);
        bad(lines);
        let mut lines = original.clone();
        lines[1][0] = 0.into();
        bad(lines);
        let mut lines = original.clone();
        lines[1][1]["node"] = serde_json::Value::Null;
        bad(lines);
        let mut lines = original.clone();
        lines.last_mut().unwrap()[1]["node"]["Def"]["kind"]["Let"]["pat"]["id"]["index"] =
            42.into();
        bad(lines);
        // Disconnected record, otherwise the expected record sequence.
        let mut lines = original.clone();
        lines[0]["nodes"] = 4.into();
        lines.insert(
            1,
            serde_json::json!([3, {"node": {"Value": "Unit"}, "children": []}]),
        );
        bad(lines);
        assert!(DefinitionSnapshot::read(&wire[..wire.len() / 2]).is_err());
        let mut trailing = wire;
        trailing.extend_from_slice(b"trailing");
        assert!(DefinitionSnapshot::read(trailing.as_slice()).is_err());
    }

    #[test]
    fn a_failed_write_releases_flat_records_on_the_same_small_stack() {
        struct Fail;
        impl Write for Fail {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let mut exp = Exp::empty();
                for _ in 0..10_000 {
                    exp = Node::dummy(ExpKind::Return(Box::new(exp)));
                }
                let snapshot = DefinitionSnapshot::new(let_def(Pat::empty(), exp));
                assert!(snapshot.write(Fail, &SimpleInterner::new()).is_err());
                // RAII destroys the original 10k-deep payload iteratively.
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
