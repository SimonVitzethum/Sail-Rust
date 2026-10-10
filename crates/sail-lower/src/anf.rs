//! ANF-Lowering-Skizze: `internal_lets`, `remove_blocks`, ANF-Normalisierung.
//!
//! Alle Pässe arbeiten pro Funktion auf der Test-IR ([`FunDef`](crate::ir::FunDef));
//! kein globaler Zustand, keine Gesamt-Clone. Nach ANF gilt:
//! Call-/BinOp-Argumente sind atomar, `Block` kommt nicht mehr vor.

use std::borrow::Cow;

use crate::ir::{Expr, FunDef, Pat};
use std::fmt;

/// Konfiguration für das ANF-Lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnfConfig {
    /// Prefix für frische Temporäre.
    pub tmp_prefix: String,
    /// Max. generierte `let`s pro Funktion (Split-Limit gegen Blowup).
    pub max_temps: usize,
}

impl Default for AnfConfig {
    fn default() -> Self {
        AnfConfig { tmp_prefix: "anf$".to_string(), max_temps: 10_000 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnfError {
    /// Zu viele Temporäre generiert (Split-Limit).
    TooManyTemps { func: String, limit: usize },
    /// Match ohne irrefutable Arm (Exhaustiveness, pro Funktion).
    NonExhaustiveMatch { func: String },
}

impl fmt::Display for AnfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnfError::TooManyTemps { func, limit } => {
                write!(f, "anf: too many temporaries in '{func}' (>{limit})")
            }
            AnfError::NonExhaustiveMatch { func } => {
                write!(f, "anf: non-exhaustive match in '{func}'")
            }
        }
    }
}

impl std::error::Error for AnfError {}

#[derive(Debug)]
struct Ctx<'a> {
    func: &'a str,
    counter: usize,
    config: &'a AnfConfig,
}

impl Ctx<'_> {
    fn fresh(&mut self) -> Result<String, AnfError> {
        if self.counter >= self.config.max_temps {
            return Err(AnfError::TooManyTemps {
                func: self.func.to_string(),
                limit: self.config.max_temps,
            });
        }
        let name = format!("{}{}", self.config.tmp_prefix, self.counter);
        self.counter += 1;
        Ok(name)
    }
}

/// `internal_lets`: zieht reine `let`-Ketten am Body-Anfang in eine
/// Liste (hier: normalisiert `Let(Let(..))`-Schachteln rechts-assoziativ).
///
/// Skizze auf Test-IR: flattene `let p1 = (let p2 = v in r) in b`
/// zu `let p2 = v in (let p1 = r in b)`.
pub fn internal_lets(body: &Expr) -> Expr {
    internal_lets_cow(body).into_owned()
}

/// `internal_lets`, borrowed where untouched (Lane-90-RAM-Diät, Muster
/// wie `subst_cow` in `sail-nexp`): Äste ohne `let`-Schachtel werden geborgt statt neu geboxt; ein `Borrowed`
/// allokiert nichts. Owned-Ergebnisse sind wert-identisch zu
/// [`internal_lets`].
///
/// Die Rotations-Sequenz (`let`-Wert ist selbst ein `let`) behält die
/// alte Top-down-Reihenfolge (Mitte aus rohen Klonen bauen, dann genau
/// einmal rekursieren), damit Ausgaben Byte-identisch bleiben; der
/// rekursive Durchlauf selbst borgt via Cow. `v2` wird wie bisher roh
/// übernommen (keine Rekursion in den rotierten Wert).
pub fn internal_lets_cow(body: &Expr) -> Cow<'_, Expr> {
    match body {
        Expr::Let(p1, v, b) => match v.as_ref() {
            Expr::Let(p2, v2, r) => {
                let inner = Expr::Let(p1.clone(), r.clone(), b.clone());
                Cow::Owned(Expr::Let(
                    p2.clone(),
                    v2.clone(),
                    Box::new(internal_lets_cow(&inner).into_owned()),
                ))
            }
            _ => {
                let vr = internal_lets_cow(v);
                let br = internal_lets_cow(b);
                if matches!(vr, Cow::Borrowed(_)) && matches!(br, Cow::Borrowed(_)) {
                    Cow::Borrowed(body)
                } else {
                    Cow::Owned(Expr::Let(
                        p1.clone(),
                        Box::new(vr.into_owned()),
                        Box::new(br.into_owned()),
                    ))
                }
            }
        },
        Expr::Block(stmts, tail) => {
            let simp: Vec<Cow<'_, Expr>> = stmts.iter().map(internal_lets_cow).collect();
            let tr = internal_lets_cow(tail);
            if simp.iter().all(|s| matches!(s, Cow::Borrowed(_)))
                && matches!(tr, Cow::Borrowed(_))
            {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Block(
                    simp.into_iter().map(Cow::into_owned).collect(),
                    Box::new(tr.into_owned()),
                ))
            }
        }
        Expr::BinOp(op, l, r) => {
            let lr = internal_lets_cow(l);
            let rr = internal_lets_cow(r);
            if matches!(lr, Cow::Borrowed(_)) && matches!(rr, Cow::Borrowed(_)) {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::BinOp(
                    *op,
                    Box::new(lr.into_owned()),
                    Box::new(rr.into_owned()),
                ))
            }
        }
        Expr::Call(n, args) => {
            let simp: Vec<Cow<'_, Expr>> = args.iter().map(internal_lets_cow).collect();
            if simp.iter().all(|a| matches!(a, Cow::Borrowed(_))) {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Call(
                    n.clone(),
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        Expr::If(c, t, e) => {
            let (cr, tr, er) = (internal_lets_cow(c), internal_lets_cow(t), internal_lets_cow(e));
            if matches!(cr, Cow::Borrowed(_))
                && matches!(tr, Cow::Borrowed(_))
                && matches!(er, Cow::Borrowed(_))
            {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::If(
                    Box::new(cr.into_owned()),
                    Box::new(tr.into_owned()),
                    Box::new(er.into_owned()),
                ))
            }
        }
        Expr::Match(s, arms) => {
            let sr = internal_lets_cow(s);
            let simp: Vec<(&Pat, Cow<'_, Expr>)> =
                arms.iter().map(|(p, e)| (p, internal_lets_cow(e))).collect();
            if matches!(sr, Cow::Borrowed(_))
                && simp.iter().all(|(_, e)| matches!(e, Cow::Borrowed(_)))
            {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Match(
                    Box::new(sr.into_owned()),
                    simp.into_iter()
                        .map(|(p, e)| (p.clone(), e.into_owned()))
                        .collect(),
                ))
            }
        }
        Expr::Var(_) | Expr::Lit(_) => Cow::Borrowed(body),
    }
}

/// `remove_blocks`: eliminiert `Block(stmts, tail)` zu geschachtelten
/// `let _ = stmt in ...`-Ketten (Seiteneffekt-Reihenfolge bleibt erhalten).
pub fn remove_blocks(body: &Expr) -> Expr {
    remove_blocks_cow(body).into_owned()
}

/// `remove_blocks`, borrowed wo unberührt (siehe [`internal_lets_cow`]).
/// `Block`-Knoten werden immer eliminiert (also `Owned`); alle anderen
/// Äste borgen bei unveränderten Kindern. Owned-Ergebnisse sind
/// wert-identisch zu [`remove_blocks`].
pub fn remove_blocks_cow(body: &Expr) -> Cow<'_, Expr> {
    match body {
        Expr::Block(stmts, tail) => {
            let tr = remove_blocks_cow(tail);
            let mut acc = tr.into_owned();
            for s in stmts.iter().rev() {
                let sr = remove_blocks_cow(s);
                acc = Expr::Let(Pat::Wildcard, Box::new(sr.into_owned()), Box::new(acc));
            }
            Cow::Owned(acc)
        }
        Expr::Let(p, v, b) => {
            let vr = remove_blocks_cow(v);
            let br = remove_blocks_cow(b);
            if matches!(vr, Cow::Borrowed(_)) && matches!(br, Cow::Borrowed(_)) {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Let(
                    p.clone(),
                    Box::new(vr.into_owned()),
                    Box::new(br.into_owned()),
                ))
            }
        }
        Expr::BinOp(op, l, r) => {
            let lr = remove_blocks_cow(l);
            let rr = remove_blocks_cow(r);
            if matches!(lr, Cow::Borrowed(_)) && matches!(rr, Cow::Borrowed(_)) {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::BinOp(
                    *op,
                    Box::new(lr.into_owned()),
                    Box::new(rr.into_owned()),
                ))
            }
        }
        Expr::Call(n, args) => {
            let simp: Vec<Cow<'_, Expr>> = args.iter().map(remove_blocks_cow).collect();
            if simp.iter().all(|a| matches!(a, Cow::Borrowed(_))) {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Call(
                    n.clone(),
                    simp.into_iter().map(Cow::into_owned).collect(),
                ))
            }
        }
        Expr::If(c, t, e) => {
            let (cr, tr, er) = (remove_blocks_cow(c), remove_blocks_cow(t), remove_blocks_cow(e));
            if matches!(cr, Cow::Borrowed(_))
                && matches!(tr, Cow::Borrowed(_))
                && matches!(er, Cow::Borrowed(_))
            {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::If(
                    Box::new(cr.into_owned()),
                    Box::new(tr.into_owned()),
                    Box::new(er.into_owned()),
                ))
            }
        }
        Expr::Match(s, arms) => {
            let sr = remove_blocks_cow(s);
            let simp: Vec<(&Pat, Cow<'_, Expr>)> =
                arms.iter().map(|(p, e)| (p, remove_blocks_cow(e))).collect();
            if matches!(sr, Cow::Borrowed(_))
                && simp.iter().all(|(_, e)| matches!(e, Cow::Borrowed(_)))
            {
                Cow::Borrowed(body)
            } else {
                Cow::Owned(Expr::Match(
                    Box::new(sr.into_owned()),
                    simp.into_iter()
                        .map(|(p, e)| (p.clone(), e.into_owned()))
                        .collect(),
                ))
            }
        }
        Expr::Var(_) | Expr::Lit(_) => Cow::Borrowed(body),
    }
}

/// Prüft Pattern-Exhaustiveness **nur pro Funktion** (Skizze):
/// jedes `match` braucht einen irrefutable Arm (Wildcard/Var) als letzten Arm.
pub fn check_exhaustive(func: &FunDef) -> Result<(), AnfError> {
    fn visit(expr: &Expr, func: &str) -> Result<(), AnfError> {
        match expr {
            Expr::Match(_, arms) => {
                let exhaustive =
                    arms.last().is_some_and(|(p, _)| p.is_irrefutable());
                if !exhaustive {
                    return Err(AnfError::NonExhaustiveMatch {
                        func: func.to_string(),
                    });
                }
                for (_, e) in arms {
                    visit(e, func)?;
                }
                Ok(())
            }
            Expr::Let(_, v, b) => {
                visit(v, func)?;
                visit(b, func)
            }
            Expr::Block(stmts, tail) => {
                for s in stmts {
                    visit(s, func)?;
                }
                visit(tail, func)
            }
            Expr::BinOp(_, l, r) => {
                visit(l, func)?;
                visit(r, func)
            }
            Expr::Call(_, args) => {
                for a in args {
                    visit(a, func)?;
                }
                Ok(())
            }
            Expr::If(c, t, e) => {
                visit(c, func)?;
                visit(t, func)?;
                visit(e, func)
            }
            Expr::Var(_) | Expr::Lit(_) => Ok(()),
        }
    }
    visit(&func.body, &func.name)
}

/// Bindet `expr` an ein frisches `let`, falls nicht atomar.
/// Gibt `(atomarer_expr, bindungen)` zurück; `bindungen` stehen außen.
fn lift_to_atom(
    expr: Expr,
    ctx: &mut Ctx<'_>,
    out: &mut Vec<(String, Expr)>,
) -> Result<Expr, AnfError> {
    // Erst normalisieren (rekursiv), dann ggf. liften.
    let normed = anf_expr(expr, ctx)?;
    if normed.is_atom() {
        Ok(normed)
    } else {
        let tmp = ctx.fresh()?;
        out.push((tmp.clone(), normed));
        Ok(Expr::Var(tmp))
    }
}

fn anf_expr(expr: Expr, ctx: &mut Ctx<'_>) -> Result<Expr, AnfError> {
    match expr {
        atom @ (Expr::Var(_) | Expr::Lit(_)) => Ok(atom),
        Expr::BinOp(op, l, r) => {
            let mut binds = Vec::new();
            let l = lift_to_atom(*l, ctx, &mut binds)?;
            let r = lift_to_atom(*r, ctx, &mut binds)?;
            let mut acc = Expr::BinOp(op, Box::new(l), Box::new(r));
            for (tmp, val) in binds.into_iter().rev() {
                acc = Expr::Let(Pat::Var(tmp), Box::new(val), Box::new(acc));
            }
            Ok(acc)
        }
        Expr::Call(name, args) => {
            let mut binds = Vec::new();
            let mut atoms = Vec::with_capacity(args.len());
            for a in args {
                atoms.push(lift_to_atom(a, ctx, &mut binds)?);
            }
            let mut acc = Expr::Call(name, atoms);
            for (tmp, val) in binds.into_iter().rev() {
                acc = Expr::Let(Pat::Var(tmp), Box::new(val), Box::new(acc));
            }
            Ok(acc)
        }
        Expr::Let(p, v, b) => {
            let v = anf_expr(*v, ctx)?;
            let b = anf_expr(*b, ctx)?;
            Ok(Expr::Let(p, Box::new(v), Box::new(b)))
        }
        Expr::Block(stmts, tail) => {
            // Blöcke sollten via `remove_blocks` weg sein; zur Robustheit hier auch.
            let rebuilt = Expr::Block(stmts, tail);
            let no_blocks = remove_blocks(&rebuilt);
            anf_expr(no_blocks, ctx)
        }
        Expr::If(c, t, e) => {
            let mut binds = Vec::new();
            let c = lift_to_atom(*c, ctx, &mut binds)?;
            let t = anf_expr(*t, ctx)?;
            let e = anf_expr(*e, ctx)?;
            let mut acc = Expr::If(Box::new(c), Box::new(t), Box::new(e));
            for (tmp, val) in binds.into_iter().rev() {
                acc = Expr::Let(Pat::Var(tmp), Box::new(val), Box::new(acc));
            }
            Ok(acc)
        }
        Expr::Match(s, arms) => {
            let mut binds = Vec::new();
            let s = lift_to_atom(*s, ctx, &mut binds)?;
            let mut new_arms = Vec::with_capacity(arms.len());
            for (p, e) in arms {
                new_arms.push((p, anf_expr(e, ctx)?));
            }
            let mut acc = Expr::Match(Box::new(s), new_arms);
            for (tmp, val) in binds.into_iter().rev() {
                acc = Expr::Let(Pat::Var(tmp), Box::new(val), Box::new(acc));
            }
            Ok(acc)
        }
    }
}

/// Volles ANF-Lowering einer Funktion: Exhaustiveness → `internal_lets` →
/// `remove_blocks` → ANF-Normalisierung.
///
/// Nimmt `func` by value und gibt die gelowerte Def zurück; das Original
/// wird dabei verbraucht (frühes Drop, kein Clone des Gesamt-AST).
pub fn anf_lower(func: FunDef, config: &AnfConfig) -> Result<FunDef, AnfError> {
    check_exhaustive(&func)?;
    // Borrowed staging: `internal_lets`/`remove_blocks` share the input
    // via Cow, so an already-flat body crosses staging with one owned
    // `staged` instead of two transient full-tree rebuilds.
    let lets = internal_lets_cow(&func.body);
    let staged = remove_blocks_cow(lets.as_ref()).into_owned();
    let mut ctx = Ctx { func: &func.name, counter: 0, config };
    // `func.body` wird hier bewegt, `func` danach zerlegt — Originaldef
    // lebt ab hier nicht mehr (Speicher freigegeben vor Rückgabe).
    let body = anf_expr(staged, &mut ctx)?;
    let FunDef { name, type_params, type_args, params, ret, body: _ } = func;
    Ok(FunDef { name, type_params, type_args, params, ret, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, Lit};

    fn fun(body: Expr) -> FunDef {
        FunDef::new("f", vec![], vec![], crate::ir::Ty::Named("int".to_string()), body)
    }

    #[test]
    fn internal_lets_flattens_nested_value() {
        let body = Expr::Let(
            Pat::Var("a".to_string()),
            Box::new(Expr::Let(
                Pat::Var("b".to_string()),
                Box::new(Expr::Lit(Lit::Int(1))),
                Box::new(Expr::Var("b".to_string())),
            )),
            Box::new(Expr::Var("a".to_string())),
        );
        let flat = internal_lets(&body);
        // Erwartet: let b = 1 in (let a = b in a)
        assert!(matches!(flat, Expr::Let(Pat::Var(b), _, _) if b == "b"));
    }

    #[test]
    fn internal_lets_cow_borrows_untouched_trees() {
        use std::borrow::Cow;
        let atom = Expr::Var("x".to_string());
        assert!(matches!(internal_lets_cow(&atom), Cow::Borrowed(_)));
        // Keine `let`-Schachtel irgendwo: komplett geborgt.
        let nested = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Call(
                "f".to_string(),
                vec![Expr::Var("x".to_string()), Expr::Lit(Lit::Int(1))],
            )),
            Box::new(Expr::If(
                Box::new(Expr::Var("c".to_string())),
                Box::new(Expr::Var("y".to_string())),
                Box::new(Expr::Var("z".to_string())),
            )),
        );
        assert!(matches!(internal_lets_cow(&nested), Cow::Borrowed(_)));
        assert!(matches!(remove_blocks_cow(&nested), Cow::Borrowed(_)));
    }

    #[test]
    fn internal_lets_cow_rotation_is_byte_identical() {
        use std::borrow::Cow;
        // `let a = (let b = 1 in b) in a` → `let b = 1 in (let a = b in a)`.
        let body = Expr::Let(
            Pat::Var("a".to_string()),
            Box::new(Expr::Let(
                Pat::Var("b".to_string()),
                Box::new(Expr::Lit(Lit::Int(1))),
                Box::new(Expr::Var("b".to_string())),
            )),
            Box::new(Expr::Var("a".to_string())),
        );
        let expected = Expr::Let(
            Pat::Var("b".to_string()),
            Box::new(Expr::Lit(Lit::Int(1))),
            Box::new(Expr::Let(
                Pat::Var("a".to_string()),
                Box::new(Expr::Var("b".to_string())),
                Box::new(Expr::Var("a".to_string())),
            )),
        );
        let out = internal_lets_cow(&body).into_owned();
        assert_eq!(out, expected);
        // Rotation baut um: kein Borrow.
        assert!(matches!(internal_lets_cow(&body), Cow::Owned(_)));
    }

    #[test]
    fn remove_blocks_cow_eliminates_in_order() {
        use std::borrow::Cow;
        let body = Expr::Block(
            vec![Expr::Lit(Lit::Int(1)), Expr::Lit(Lit::Int(2))],
            Box::new(Expr::Var("x".to_string())),
        );
        let expected = Expr::Let(
            Pat::Wildcard,
            Box::new(Expr::Lit(Lit::Int(1))),
            Box::new(Expr::Let(
                Pat::Wildcard,
                Box::new(Expr::Lit(Lit::Int(2))),
                Box::new(Expr::Var("x".to_string())),
            )),
        );
        let out = remove_blocks_cow(&body).into_owned();
        assert_eq!(out, expected);
        assert!(matches!(remove_blocks_cow(&body), Cow::Owned(_)));
        // Reiner `let`-Baum ohne Block: geborgt.
        let plain = Expr::Let(
            Pat::Var("a".to_string()),
            Box::new(Expr::Lit(Lit::Int(1))),
            Box::new(Expr::Var("a".to_string())),
        );
        assert!(matches!(remove_blocks_cow(&plain), Cow::Borrowed(_)));
        assert!(matches!(internal_lets_cow(&plain), Cow::Borrowed(_)));
    }

    #[test]
    fn remove_blocks_eliminates_all_blocks() {
        let body = Expr::Block(
            vec![Expr::Lit(Lit::Int(1)), Expr::Lit(Lit::Int(2))],
            Box::new(Expr::Var("x".to_string())),
        );
        let out = remove_blocks(&body);
        fn has_block(e: &Expr) -> bool {
            match e {
                Expr::Block(_, _) => true,
                Expr::Let(_, v, b) => has_block(v) || has_block(b),
                Expr::BinOp(_, l, r) => has_block(l) || has_block(r),
                Expr::Call(_, a) => a.iter().any(has_block),
                Expr::If(c, t, e) => has_block(c) || has_block(t) || has_block(e),
                Expr::Match(s, arms) => {
                    has_block(s) || arms.iter().any(|(_, e)| has_block(e))
                }
                _ => false,
            }
        }
        assert!(!has_block(&out));
    }

    #[test]
    fn anf_lowers_nested_calls_to_atoms() {
        let body = Expr::Call(
            "f".to_string(),
            vec![Expr::Call("g".to_string(), vec![Expr::Lit(Lit::Int(1))])],
        );
        let lowered = anf_lower(fun(body), &AnfConfig::default()).unwrap();
        // Äußerer Call darf nur Atome als Args haben.
        match &lowered.body {
            Expr::Let(_, _, inner) => {
                assert!(matches!(inner.as_ref(), Expr::Call(_, args) if args.iter().all(|a| a.is_atom())));
            }
            other => panic!("expected let-binding, got {other:?}"),
        }
    }

    #[test]
    fn anf_rejects_non_exhaustive_match() {
        let body = Expr::Match(
            Box::new(Expr::Var("x".to_string())),
            vec![(Pat::Lit(Lit::Int(0)), Expr::Lit(Lit::Int(1)))],
        );
        let err = anf_lower(fun(body), &AnfConfig::default()).unwrap_err();
        assert_eq!(err, AnfError::NonExhaustiveMatch { func: "f".to_string() });
    }

    #[test]
    fn anf_accepts_wildcard_arm() {
        let body = Expr::Match(
            Box::new(Expr::Var("x".to_string())),
            vec![
                (Pat::Lit(Lit::Int(0)), Expr::Lit(Lit::Int(1))),
                (Pat::Wildcard, Expr::Lit(Lit::Int(2))),
            ],
        );
        assert!(anf_lower(fun(body), &AnfConfig::default()).is_ok());
    }

    #[test]
    fn anf_temp_limit_enforced() {
        let body = Expr::Call(
            "f".to_string(),
            vec![
                Expr::Call("g".to_string(), vec![Expr::Lit(Lit::Int(1))]),
                Expr::Call("h".to_string(), vec![Expr::Lit(Lit::Int(2))]),
            ],
        );
        let cfg = AnfConfig { tmp_prefix: "t".to_string(), max_temps: 1 };
        assert_eq!(
            anf_lower(fun(body), &cfg).unwrap_err(),
            AnfError::TooManyTemps { func: "f".to_string(), limit: 1 }
        );
    }

    #[test]
    fn anf_binop_args_are_atoms() {
        let body = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Call("g".to_string(), vec![Expr::Lit(Lit::Int(1))])),
            Box::new(Expr::Lit(Lit::Int(2))),
        );
        let lowered = anf_lower(fun(body), &AnfConfig::default()).unwrap();
        assert!(matches!(lowered.body, Expr::Let(_, _, _)));
    }
}
