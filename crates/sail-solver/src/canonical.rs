//! Stack-safe canonical Sail constraints -> validated SMT-LIB. No AST clones,
//! integer truncation, raw Sail identifiers, or unproved opaque fallbacks.
use sail_ast::*;
use std::collections::BTreeMap;
use std::fmt::Write;

/// Binder identities belong to a typing scope, not an interner table.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub interner: &'a SimpleInterner,
    pub variables: &'a BTreeMap<String, u64>,
}
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub constraint: &'a Constraint,
    pub context: Context<'a>,
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
            output: String::new(),
        };
        for assumption in assumptions {
            emitter.output.push_str("(assert ");
            emitter.emit(
                Task::Constraint(assumption.constraint, assumption.context, false),
                None,
            )?;
            emitter.output.push_str(")\n");
        }
        emitter.output.push_str("(assert (not ");
        emitter.emit(
            Task::Constraint(goal.constraint, goal.context, true),
            substitution,
        )?;
        emitter.output.push_str("))\n");
        let mut script = String::new();
        for (name, sort) in emitter.declarations {
            writeln!(script, "(declare-const {name} {sort})").unwrap();
        }
        script.push_str(&emitter.output);
        Ok(Self { script })
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
        let key = if variable {
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
                    NExpKind::Exp(_) | NExpKind::App { .. } => {
                        return Err(
                            "numeric application/power needs elaboration before proof".into()
                        )
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
                        constraint: &constraint,
                        context: Context {
                            interner: &it,
                            variables: &variables,
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
                constraint: &c,
                context: Context {
                    interner: &it,
                    variables: &variables
                }
            },
            None
        )
        .is_err());
    }
}
