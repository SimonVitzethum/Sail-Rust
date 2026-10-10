//! `-auto_mono`-Äquivalent (Stub mit Split-Limits).
//!
//! Ersetzt Typvariablen einer polymorphen [`FunDef`](crate::ir::FunDef)
//! durch konkrete [`Ty`](crate::ir::Ty)-Argumente. Arbeitet strikt
//! pro Funktion: keine globale Instanz-Tabelle, keine programweiten Fixpunkte.
//! Split-Limits verhindern Blowup (vgl. OCaml `-auto_mono` + Split-Fuel).

use std::borrow::Cow;

use crate::ir::{Expr, FunDef, Ty};
use std::collections::BTreeMap;
use std::fmt;

/// Konfiguration für die Monomorphisierung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoConfig {
    /// Max. Instanzen pro generischer Funktion (Split-Limit).
    pub max_specializations: usize,
    /// Max. Knoten pro spezialisiertem Body (Split-Limit).
    pub max_body_size: usize,
}

impl Default for MonoConfig {
    fn default() -> Self {
        MonoConfig {
            max_specializations: 32,
            max_body_size: 10_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonoError {
    /// Mehr Instanzen angefragt als `max_specializations`.
    SplitLimitExceeded { func: String, limit: usize },
    /// Spezialisierter Body überschreitet `max_body_size`.
    BodyTooLarge { func: String, size: usize, limit: usize },
    /// Arity der Typ-Argumente passt nicht zu `type_params`.
    ArityMismatch {
        func: String,
        expected: usize,
        got: usize,
    },
    /// Monomorphe Funktion braucht keine Instanziierung.
    NotPolymorphic { func: String },
}

impl fmt::Display for MonoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MonoError::SplitLimitExceeded { func, limit } => {
                write!(f, "mono: split limit exceeded for '{func}' (>{limit} specializations)")
            }
            MonoError::BodyTooLarge { func, size, limit } => {
                write!(f, "mono: body of '{func}' too large ({size}>{limit} nodes)")
            }
            MonoError::ArityMismatch { func, expected, got } => {
                write!(
                    f,
                    "mono: arity mismatch for '{func}' (expected {expected}, got {got})"
                )
            }
            MonoError::NotPolymorphic { func } => {
                write!(f, "mono: function '{func}' is not polymorphic")
            }
        }
    }
}

impl std::error::Error for MonoError {}

fn subst_ty_cow<'a>(ty: &'a Ty, env: &BTreeMap<&str, &Ty>) -> Cow<'a, Ty> {
    match ty {
        // Treffer: einziger Clone (das eingesetzte `Ty`); Miss und
        // Nicht-Var borgen (RAM-Diät Lane 90, Muster wie `subst_cow`).
        Ty::Var(v) => match env.get(v.as_str()) {
            Some(t) => Cow::Owned((*t).clone()),
            None => Cow::Borrowed(ty),
        },
        _ => Cow::Borrowed(ty),
    }
}

fn subst_ty(ty: &Ty, env: &BTreeMap<&str, &Ty>) -> Ty {
    subst_ty_cow(ty, env).into_owned()
}

fn subst_expr_cow<'a>(expr: &'a Expr, env: &BTreeMap<&str, &Ty>) -> Cow<'a, Expr> {
    // Test-IR: Typen stehen nur in Let-Patterns nicht; Calls/Vars bleiben.
    // Wir traversieren trotzdem (Platzhalter für echte typ-annotierte IR),
    // damit Split-Limit über `size` greift. `env` ist hier ungenutzt,
    // wird in der echten IR zur Substitution.
    //
    // Da (noch) nichts zu ersetzen ist, wird immer geborgt: das alte
    // `expr.clone()` pro `monomorphize_one` entfällt ersatzlos.
    let _ = env;
    Cow::Borrowed(expr)
}

fn subst_expr(expr: &Expr, env: &BTreeMap<&str, &Ty>) -> Expr {
    subst_expr_cow(expr, env).into_owned()
}

/// Instanziiert `func` für genau eine Typ-Argument-Liste.
///
/// Gibt die spezialisierte (monomorphe) [`FunDef`] zurück. Der Name wird
/// deterministisch suffixiert (`name_T1_T2`), damit Shards ohne globale
/// Tabelle auskommen.
pub fn monomorphize_one(
    func: &FunDef,
    type_args: &[Ty],
    _config: &MonoConfig,
) -> Result<FunDef, MonoError> {
    if func.type_params.is_empty() {
        return Err(MonoError::NotPolymorphic {
            func: func.name.clone(),
        });
    }
    if type_args.len() != func.type_params.len() {
        return Err(MonoError::ArityMismatch {
            func: func.name.clone(),
            expected: func.type_params.len(),
            got: type_args.len(),
        });
    }
    let env: BTreeMap<&str, &Ty> = func
        .type_params
        .iter()
        .map(String::as_str)
        .zip(type_args.iter())
        .collect();

    // Deterministischer Namens-Suffix ohne Zwischen-`Vec`: jeder
    // Typ wird einmal gerendert (`to_string`), Klammern/Kommas werden
    // beim Hineinschieben ersetzt (statt `replace`+`join`-Kette mit je
    // zwei Alloks pro Typ). Ausgabe identisch zur alten Kette.
    let mut suffix = String::with_capacity(type_args.len() * 8);
    for (i, t) in type_args.iter().enumerate() {
        if i > 0 {
            suffix.push('_');
        }
        for ch in t.to_string().chars() {
            match ch {
                '(' | ')' | ',' => suffix.push('_'),
                _ => suffix.push(ch),
            }
        }
    }
    let mut params = Vec::with_capacity(func.params.len());
    params.extend(func.params.iter().map(|(n, t)| (n.clone(), subst_ty(t, &env))));
    let body = subst_expr(&func.body, &env);
    Ok(FunDef {
        name: format!("{}#{suffix}", func.name),
        type_params: Vec::new(),
        type_args: Some(type_args.to_vec()),
        params,
        ret: subst_ty(&func.ret, &env),
        body,
    })
}

/// Monomorphisiert `func` für alle `instance_sets`.
///
/// Enforcet beide Split-Limits: Anzahl Instanzen und Body-Größe je Instanz.
pub fn monomorphize(
    func: &FunDef,
    instance_sets: &[Vec<Ty>],
    config: &MonoConfig,
) -> Result<Vec<FunDef>, MonoError> {
    if instance_sets.len() > config.max_specializations {
        return Err(MonoError::SplitLimitExceeded {
            func: func.name.clone(),
            limit: config.max_specializations,
        });
    }
    instance_sets
        .iter()
        .map(|args| {
            let spec = monomorphize_one(func, args, config)?;
            let size = spec.body.size();
            if size > config.max_body_size {
                return Err(MonoError::BodyTooLarge {
                    func: spec.name.clone(),
                    size,
                    limit: config.max_body_size,
                });
            }
            Ok(spec)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Expr, Lit};

    fn poly_id() -> FunDef {
        FunDef::new(
            "id",
            vec!["a".to_string()],
            vec![("x".to_string(), Ty::Var("a".to_string()))],
            Ty::Var("a".to_string()),
            Expr::Var("x".to_string()),
        )
    }

    #[test]
    fn mono_single_instance() {
        let f = poly_id();
        let out = monomorphize(&f, &[vec![Ty::Named("int".to_string())]], &MonoConfig::default())
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].type_params, Vec::<String>::new());
        assert_eq!(out[0].params[0].1, Ty::Named("int".to_string()));
        assert_eq!(out[0].ret, Ty::Named("int".to_string()));
    }

    #[test]
    fn mono_split_limit_enforced() {
        let f = poly_id();
        let cfg = MonoConfig { max_specializations: 1, max_body_size: 10_000 };
        let sets = vec![
            vec![Ty::Named("int".to_string())],
            vec![Ty::Named("bool".to_string())],
        ];
        assert_eq!(
            monomorphize(&f, &sets, &cfg),
            Err(MonoError::SplitLimitExceeded { func: "id".to_string(), limit: 1 })
        );
    }

    #[test]
    fn mono_arity_mismatch() {
        let f = poly_id();
        let err = monomorphize(&f, &[vec![]], &MonoConfig::default()).unwrap_err();
        assert_eq!(
            err,
            MonoError::ArityMismatch { func: "id".to_string(), expected: 1, got: 0 }
        );
    }

    #[test]
    fn mono_body_size_limit() {
        let big = FunDef::new(
            "big",
            vec!["a".to_string()],
            vec![],
            Ty::Var("a".to_string()),
            Expr::Block(
                (0..10).map(|i| Expr::Lit(Lit::Int(i))).collect(),
                Box::new(Expr::Lit(Lit::Unit)),
            ),
        );
        let cfg = MonoConfig { max_specializations: 8, max_body_size: 2 };
        assert!(matches!(
            monomorphize(&big, &[vec![Ty::Named("int".to_string())]], &cfg),
            Err(MonoError::BodyTooLarge { .. })
        ));
    }

    #[test]
    fn mono_monomorphic_rejected() {
        let f = FunDef::new(
            "f",
            vec![],
            vec![],
            Ty::Named("int".to_string()),
            Expr::Lit(Lit::Int(1)),
        );
        assert_eq!(
            monomorphize(&f, &[vec![Ty::Named("int".to_string())]], &MonoConfig::default())
                .unwrap_err(),
            MonoError::NotPolymorphic { func: "f".to_string() }
        );
    }

    #[test]
    fn mono_suffix_and_subst_cow_contract() {
        use std::borrow::Cow;
        let f = poly_id();
        // Suffix-Format: `name_T1_T2`, Klammern/Kommas → `_`.
        let out = monomorphize_one(&f, &[Ty::Bits(8)], &MonoConfig::default()).unwrap();
        assert_eq!(out.name, "id#bits_8_");
        let out = monomorphize_one(
            &f,
            &[Ty::Named("int".to_string()), Ty::Bits(8)],
            &MonoConfig::default(),
        );
        // `id` hat genau einen Typparameter: Arity-Fehler, kein Suffix-Raten.
        assert!(out.is_err());
        // `subst_ty_cow`: Treffer → Owned, Miss/Nicht-Var → Borrowed.
        let env: BTreeMap<&str, &Ty> = BTreeMap::new();
        let v = Ty::Var("a".to_string());
        assert!(matches!(subst_ty_cow(&v, &env), Cow::Borrowed(_)));
        assert!(matches!(
            subst_ty_cow(&Ty::Named("int".to_string()), &env),
            Cow::Borrowed(_)
        ));
        let int = Ty::Named("int".to_string());
        let env: BTreeMap<&str, &Ty> = [("a", &int)].into_iter().collect();
        assert_eq!(subst_ty_cow(&v, &env).into_owned(), int);
        // `subst_expr_cow`: Test-IR ohne Typen in Exprs → immer Borrowed.
        let body = Expr::Var("x".to_string());
        assert!(matches!(subst_expr_cow(&body, &env), Cow::Borrowed(_)));
        assert_eq!(subst_expr(&body, &env), body);
    }
}
