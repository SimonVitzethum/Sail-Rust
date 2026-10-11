#![forbid(unsafe_code)]
//! Execute source and lowered assignments to check effects and aliasing.
use std::collections::BTreeMap;
use sail_rewrite::{Def, DefKind, Pipeline, Shard, Stmt};

#[derive(Clone, Debug, PartialEq)]
enum Value { Int(i32), Tuple(Vec<Value>) }
#[derive(Clone, Debug, PartialEq)]
struct Machine { vars: BTreeMap<String, Value>, calls: usize, writes: usize }
impl Machine {
    fn eval(&mut self, rhs: &str) -> Value {
        if rhs == "next_pair()" {
            self.calls += 1;
            return Value::Tuple(vec![Value::Int(10), Value::Int(20)]);
        }
        if let Some(inner) = rhs.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            return Value::Tuple(inner.split(',').map(|s| self.eval(s.trim())).collect());
        }
        self.vars[rhs].clone()
    }
    fn run(&mut self, body: &[Stmt]) {
        for stmt in body {
            match stmt {
                Stmt::Assign { lhs, rhs } => {
                    let value = self.eval(rhs);
                    self.vars.insert(lhs.clone(), value);
                    self.writes += 1;
                }
                Stmt::TupleAssign { targets, rhs } => {
                    let Value::Tuple(values) = self.eval(rhs) else { panic!("tuple expected") };
                    assert_eq!(targets.len(), values.len());
                    for (target, value) in targets.iter().zip(values) {
                        self.vars.insert(target.clone(), value);
                        self.writes += 1;
                    }
                }
                Stmt::LetTuple { bindings, rhs, body } => {
                    let Value::Tuple(values) = self.eval(rhs) else { panic!("tuple expected") };
                    assert_eq!(bindings.len(), values.len());
                    let old: Vec<_> = bindings.iter().zip(values)
                        .map(|(name, value)| (name.clone(), self.vars.insert(name.clone(), value)))
                        .collect();
                    self.run(body);
                    for (name, value) in old {
                        if let Some(value) = value { self.vars.insert(name, value); }
                        else { self.vars.remove(&name); }
                    }
                }
                _ => panic!("unexpected statement"),
            }
        }
    }
}

#[test]
fn tuple_lowering_preserves_calls_swaps_and_register_write_effects() {
    for rhs in ["next_pair()", "(b, a)", "(__sail_tuple_0, a)"] {
        let body = vec![
            Stmt::TupleAssign { targets: vec!["a".into(), "b".into()], rhs: rhs.into() },
            Stmt::Assign { lhs: "R".into(), rhs: "R".into() },
        ];
        let mut source = Machine {
            vars: [("a", 1), ("b", 2), ("R", 3), ("__sail_tuple_0", 4)].into_iter()
                .map(|(k, v)| (k.into(), Value::Int(v))).collect(),
            calls: 0, writes: 0,
        };
        let mut rewritten = source.clone();
        let out = Pipeline::from_names(&["tuple_assignments", "simple_assignments"]).unwrap()
            .run(Shard::new(0, vec![Def::new("f", DefKind::Function, body.clone())]));
        source.run(&body);
        rewritten.run(&out.defs[0].body);
        assert_eq!(source, rewritten, "semantics changed for {rhs}");
        assert_eq!(rewritten.writes, 3);
        assert_eq!(rewritten.calls, usize::from(rhs == "next_pair()"));
    }
}
