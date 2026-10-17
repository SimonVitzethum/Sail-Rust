//! Stack-safe canonical Sail constraints -> validated SMT-LIB. Powers use a
//! conservative congruence abstraction; only UNSAT proves obligations. No
//! recursive AST clones, integer truncation or raw Sail identifiers.
use sail_ast::*;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Binder identities belong to a typing scope, not an interner table.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub interner: &'a SimpleInterner,
    pub variables: &'a BTreeMap<String, u64>,
    pub bindings: Option<&'a dyn Bindings>,
}
/// Borrowed substitutions may refer to different declaration interners. The
/// emitter follows them on its work stack, without copying numeric ASTs.
pub trait Bindings {
    fn resolve(&self, symbol: Symbol) -> Option<Argument<'_>>;
    fn rigid(&self, name: &str) -> Option<(usize, Kind)>;
}
#[derive(Clone, Copy)]
pub struct Argument<'a> {
    pub argument: &'a TypArg,
    pub context: Context<'a>,
}
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub constraint: &'a Constraint,
    pub context: Context<'a>,
    /// Negate a borrowed predicate without cloning its recursive AST. This is
    /// used for the false branch of a singleton Boolean condition.
    pub negated: bool,
}
pub struct Substitution<'a> {
    pub arguments: BTreeMap<Symbol, &'a TypArg>,
    pub context: Context<'a>,
}
/// Private script field prevents callers from injecting SMT commands. The
/// script asserts assumptions and the negated obligation, never check-sat.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalQuery {
    script: String,
}
impl CanonicalQuery {
    pub fn script(&self) -> &str {
        &self.script
    }

    pub fn implication<'a>(
        assumptions: &[View<'a>],
        goal: View<'a>,
        substitution: Option<&Substitution<'a>>,
    ) -> Result<Self, String> {
        let mut emitter = Emitter {
            declarations: BTreeMap::new(),
            uses_power: false,
            output: String::new(),
        };
        for assumption in assumptions {
            emitter.output.push_str("(assert ");
            emitter.emit_view(*assumption, false, None)?;
            emitter.output.push_str(")\n");
        }
        emitter.output.push_str("(assert (not ");
        emitter.emit_view(goal, true, substitution)?;
        emitter.output.push_str("))\n");
        Ok(emitter.finish())
    }

    pub fn equality(
        assumptions: &[View<'_>],
        left: Argument<'_>,
        right: Argument<'_>,
    ) -> Result<Self, String> {
        if std::mem::discriminant(&left.argument.kind)
            != std::mem::discriminant(&right.argument.kind)
        {
            return Err("mixed proof equality sorts".into());
        }
        let mut emitter = Emitter {
            declarations: BTreeMap::new(),
            uses_power: false,
            output: String::new(),
        };
        for a in assumptions {
            emitter.output.push_str("(assert ");
            emitter.emit_view(*a, false, None)?;
            emitter.output.push_str(")\n");
        }
        emitter.output.push_str("(assert (not (= ");
        emitter.emit(Task::Arg(left.argument, left.context, false), None)?;
        emitter.output.push(' ');
        emitter.emit(Task::Arg(right.argument, right.context, false), None)?;
        emitter.output.push_str(")))\n");
        Ok(emitter.finish())
    }
}
impl Emitter {
    fn emit_view<'a>(
        &mut self,
        view: View<'a>,
        substitute: bool,
        substitution: Option<&Substitution<'a>>,
    ) -> Result<(), String> {
        if view.negated {
            self.output.push_str("(not ");
        }
        self.emit(
            Task::Constraint(view.constraint, view.context, substitute),
            substitution,
        )?;
        if view.negated {
            self.output.push(')');
        }
        Ok(())
    }
    fn finish(self) -> CanonicalQuery {
        let mut script = String::new();
        if self.uses_power {
            // A conservative abstraction: every mathematical power function
            // is an interpretation of this UF. UNSAT therefore remains sound.
            // No positivity or arithmetic axiom is invented by the emitter.
            script.push_str("(declare-fun sail_pow2 (Int) Int)\n");
        }
        for (name, sort) in self.declarations {
            writeln!(script, "(declare-const {name} {sort})").unwrap();
        }
        script.push_str(&self.output);
        CanonicalQuery { script }
    }
}
enum Task<'a> {
    Text(&'static str),
    Number(&'a str),
    Constraint(&'a Constraint, Context<'a>, bool),
    NExp(&'a NExp, Context<'a>, bool),
    Arg(&'a TypArg, Context<'a>, bool),
}
struct Emitter {
    declarations: BTreeMap<String, &'static str>,
    uses_power: bool,
    output: String,
}
fn name(it: &SimpleInterner, symbol: Symbol) -> Result<&str, String> {
    it.resolve(symbol)
        .ok_or_else(|| "unresolved constraint symbol".into())
}
fn encoded(prefix: &str, value: &str) -> String {
    let mut output = prefix.to_owned();
    for byte in value.bytes() {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
impl Emitter {
    fn identifier(
        &mut self,
        context: Context<'_>,
        symbol: Symbol,
        variable: bool,
        sort: &'static str,
    ) -> Result<(), String> {
        let text = name(context.interner, symbol)?;
        let key = if variable && context.bindings.is_some() {
            let (scope, kind) = context
                .bindings
                .unwrap()
                .rigid(text)
                .ok_or_else(|| format!("unbound rigid proof variable {text}"))?;
            if !matches!((kind, sort), (Kind::Int, "Int") | (Kind::Bool, "Bool")) {
                return Err("rigid proof variable sort mismatch".into());
            }
            encoded(&format!("scope_{scope:x}_"), text)
        } else if variable {
            format!(
                "v_{}",
                context
                    .variables
                    .get(text)
                    .ok_or_else(|| format!("unbound proof variable {text}"))?
            )
        } else {
            encoded("id_", text)
        };
        // Bool and integer names occupy distinct namespaces, including ids.
        let key = format!("{sort}_{key}");
        self.declarations.insert(key.clone(), sort);
        self.output.push_str(&key);
        Ok(())
    }
    fn emit<'a>(
        &mut self,
        initial: Task<'a>,
        substitution: Option<&Substitution<'a>>,
    ) -> Result<(), String> {
        let mut work = vec![initial];
        while let Some(task) = work.pop() {
            match task {
                Task::Text(text) => self.output.push_str(text),
                Task::Number(number) => {
                    let (negative, digits) = number
                        .strip_prefix('-')
                        .map_or((false, number), |n| (true, n));
                    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                        return Err("invalid decimal constraint literal".into());
                    }
                    let digits = digits.trim_start_matches('0');
                    let digits = if digits.is_empty() { "0" } else { digits };
                    if negative {
                        write!(self.output, "(- {digits})").unwrap();
                    } else {
                        self.output.push_str(digits);
                    }
                }
                Task::Arg(arg, context, subst) => work.push(match &arg.kind {
                    TypArgKind::NExp(n) => Task::NExp(n, context, subst),
                    TypArgKind::Bool(c) => Task::Constraint(c, context, subst),
                    TypArgKind::Typ(_) => return Err("type equality in numeric proof".into()),
                }),
                Task::NExp(n, context, subst) => match &n.kind {
                    NExpKind::Constant(n) => work.push(Task::Number(n)),
                    NExpKind::Var(symbol) => {
                        if let Some((arg, target)) = substitution
                            .filter(|_| subst)
                            .and_then(|s| s.arguments.get(symbol).map(|a| (*a, s.context)))
                        {
                            if !matches!(arg.kind, TypArgKind::NExp(_)) {
                                return Err("non-integer proof substitution".into());
                            }
                            work.push(Task::Arg(arg, target, false));
                        } else if let Some(bound) =
                            context.bindings.and_then(|b| b.resolve(*symbol))
                        {
                            if !matches!(bound.argument.kind, TypArgKind::NExp(_)) {
                                return Err("non-integer borrowed substitution".into());
                            }
                            work.push(Task::Arg(bound.argument, bound.context, false));
                        } else {
                            self.identifier(context, *symbol, true, "Int")?;
                        }
                    }
                    NExpKind::Id(id) => self.identifier(context, id.symbol, false, "Int")?,
                    NExpKind::Sum(a, b) | NExpKind::Minus(a, b) | NExpKind::Times(a, b) => {
                        self.output.push_str(match &n.kind {
                            NExpKind::Sum(..) => "(+ ",
                            NExpKind::Minus(..) => "(- ",
                            _ => "(* ",
                        });
                        work.extend([
                            Task::Text(")"),
                            Task::NExp(b, context, subst),
                            Task::Text(" "),
                            Task::NExp(a, context, subst),
                        ]);
                    }
                    NExpKind::Neg(a) => {
                        self.output.push_str("(- ");
                        work.extend([Task::Text(")"), Task::NExp(a, context, subst)]);
                    }
                    NExpKind::If {
                        cond,
                        then_branch,
                        else_branch,
                    } => {
                        self.output.push_str("(ite ");
                        work.extend([
                            Task::Text(")"),
                            Task::NExp(else_branch, context, subst),
                            Task::Text(" "),
                            Task::NExp(then_branch, context, subst),
                            Task::Text(" "),
                            Task::Constraint(cond, context, subst),
                        ]);
                    }
                    NExpKind::App { func, args }
                        if name(context.interner, func.symbol)? == "abs" && args.len() == 1 =>
                    {
                        self.output.push_str("(ite (>= ");
                        let a = &args[0];
                        work.extend([
                            Task::Text("))"),
                            Task::NExp(a, context, subst),
                            Task::Text(" (- "),
                            Task::NExp(a, context, subst),
                            Task::Text(" 0) "),
                            Task::NExp(a, context, subst),
                        ]);
                    }
                    NExpKind::Exp(exponent) => {
                        self.uses_power = true;
                        self.output.push_str("(sail_pow2 ");
                        work.extend([Task::Text(")"), Task::NExp(exponent, context, subst)]);
                    }
                    NExpKind::App { .. } => {
                        return Err("numeric application needs elaboration before proof".into())
                    }
                },
                Task::Constraint(c, context, subst) => match &c.kind {
                    ConstraintKind::True => self.output.push_str("true"),
                    ConstraintKind::False => self.output.push_str("false"),
                    ConstraintKind::Var(symbol) => {
                        if let Some((arg, target)) = substitution
                            .filter(|_| subst)
                            .and_then(|s| s.arguments.get(symbol).map(|a| (*a, s.context)))
                        {
                            if !matches!(arg.kind, TypArgKind::Bool(_)) {
                                return Err("non-Boolean proof substitution".into());
                            }
                            work.push(Task::Arg(arg, target, false));
                        } else if let Some(bound) =
                            context.bindings.and_then(|b| b.resolve(*symbol))
                        {
                            if !matches!(bound.argument.kind, TypArgKind::Bool(_)) {
                                return Err("non-Boolean borrowed substitution".into());
                            }
                            work.push(Task::Arg(bound.argument, bound.context, false));
                        } else {
                            self.identifier(context, *symbol, true, "Bool")?;
                        }
                    }
                    ConstraintKind::Id(id) => self.identifier(context, id.symbol, false, "Bool")?,
                    ConstraintKind::Equal(a, b) | ConstraintKind::NotEqual(a, b) => {
                        if std::mem::discriminant(&a.kind) != std::mem::discriminant(&b.kind) {
                            return Err("mixed proof equality sorts".into());
                        }
                        let unequal = matches!(c.kind, ConstraintKind::NotEqual(..));
                        self.output
                            .push_str(if unequal { "(not (= " } else { "(= " });
                        work.extend([
                            Task::Text(if unequal { "))" } else { ")" }),
                            Task::Arg(b, context, subst),
                            Task::Text(" "),
                            Task::Arg(a, context, subst),
                        ]);
                    }
                    ConstraintKind::Ge(a, b)
                    | ConstraintKind::Gt(a, b)
                    | ConstraintKind::Le(a, b)
                    | ConstraintKind::Lt(a, b) => {
                        self.output.push_str(match c.kind {
                            ConstraintKind::Ge(..) => "(>= ",
                            ConstraintKind::Gt(..) => "(> ",
                            ConstraintKind::Le(..) => "(<= ",
                            _ => "(< ",
                        });
                        work.extend([
                            Task::Text(")"),
                            Task::NExp(b, context, subst),
                            Task::Text(" "),
                            Task::NExp(a, context, subst),
                        ]);
                    }
                    ConstraintKind::And(a, b) | ConstraintKind::Or(a, b) => {
                        self.output
                            .push_str(if matches!(c.kind, ConstraintKind::And(..)) {
                                "(and "
                            } else {
                                "(or "
                            });
                        work.extend([
                            Task::Text(")"),
                            Task::Constraint(b, context, subst),
                            Task::Text(" "),
                            Task::Constraint(a, context, subst),
                        ]);
                    }
                    ConstraintKind::Not(a) => {
                        self.output.push_str("(not ");
                        work.extend([Task::Text(")"), Task::Constraint(a, context, subst)]);
                    }
                    ConstraintKind::Set { nexp, members } => {
                        if members.is_empty() {
                            self.output.push_str("false");
                        } else {
                            self.output.push_str("(or");
                            work.push(Task::Text(")"));
                            for member in members.iter().rev() {
                                work.extend([
                                    Task::Text(")"),
                                    Task::Number(member),
                                    Task::Text(" "),
                                    Task::NExp(nexp, context, subst),
                                    Task::Text(" (= "),
                                ]);
                            }
                        }
                    }
                    ConstraintKind::App { func, args }
                        if name(context.interner, func.symbol)? == "not" && args.len() == 1 =>
                    {
                        if !matches!(args[0].kind, TypArgKind::Bool(_)) {
                            return Err("not requires Boolean argument".into());
                        }
                        self.output.push_str("(not ");
                        work.extend([Task::Text(")"), Task::Arg(&args[0], context, subst)]);
                    }
                    ConstraintKind::App { .. } => {
                        return Err("constraint synonym needs expansion before proof".into())
                    }
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_query_handles_ten_thousand_nodes_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let it = SimpleInterner::new();
                let variables = BTreeMap::new();
                let mut n = Node::dummy(NExpKind::Constant("0".into()));
                for _ in 0..10_000 {
                    n = Node::dummy(NExpKind::Sum(
                        Box::new(n),
                        Box::new(Node::dummy(NExpKind::Constant("1".into()))),
                    ));
                }
                let constraint = Node::dummy(ConstraintKind::Ge(
                    n,
                    Node::dummy(NExpKind::Constant("0".into())),
                ));
                let query = CanonicalQuery::implication(
                    &[],
                    View {
                        negated: false,
                        constraint: &constraint,
                        context: Context {
                            interner: &it,
                            variables: &variables,
                            bindings: None,
                        },
                    },
                    None,
                )
                .unwrap();
                assert!(query.script().len() > 60_000);
                let ConstraintKind::Ge(n, _) = constraint.kind else {
                    panic!()
                };
                let mut work = vec![n];
                while let Some(n) = work.pop() {
                    if let NExpKind::Sum(a, b) = n.kind {
                        work.push(*a);
                        work.push(*b);
                    }
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn malformed_integer_literals_cannot_inject_smt_commands() {
        let it = SimpleInterner::new();
        let variables = BTreeMap::new();
        let n = Node::dummy(NExpKind::Constant("0) (assert false)".into()));
        let c = Node::dummy(ConstraintKind::Ge(
            n,
            Node::dummy(NExpKind::Constant("0".into())),
        ));
        assert!(CanonicalQuery::implication(
            &[],
            View {
                negated: false,
                constraint: &c,
                context: Context {
                    interner: &it,
                    variables: &variables,
                    bindings: None,
                }
            },
            None
        )
        .is_err());
    }
}
