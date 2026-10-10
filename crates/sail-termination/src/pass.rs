#![forbid(unsafe_code)]
//! Sketch of the `move_termination_measures` rewrite pass.
//!
//! Mirrors `move_termination_measures` in `rewrites.ml`:
//!
//! 1. Collect out-of-line `DEF_measure (id, pat, exp)` declarations into a
//!    map; a second measure for the same function is an error (OCaml raises
//!    `err_general "Second termination measure given for …"`).
//! 2. Attach each measure to its function definition as
//!    `Rec_measure (pat, exp)` (OCaml: `Rec_aux (Rec_measure (pat, exp), …)`).
//! 3. Drop the `DEF_measure` nodes from the definition list.
//! 4. Warn about measures with no matching function (OCaml prints
//!    `"unused loop measure for function …"` — here collected as warnings).
//!
//! The OCaml pass additionally hoists `val` specs of functions called in the
//! measure above the definition so the result type-checks; that ordering
//! concern is modelled by [`AttachedMeasure::hoisted_specs`].
//!
//! This module also parses the concrete directive syntax found in
//! `sail-arm/arm-v9.4-a/src/termination_measures.sail`:
//!
//! ```text
//! termination_measure Reduce__1(op,input,esize,altfp) = 'N
//! termination_measure FloorPow2
//!   while x - pow2(n)
//! termination_measure ImplementedSMEVectorLength
//!   while nbits, while maxbits - nbits
//! termination_measure AArch64_S2Walk repeat 100, repeat 100
//! ```

use crate::rec_measure::RecMeasure;

/// A loop measure inside a `termination_measure` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopMeasure {
    /// `while <expr>`: fuel expression for one loop level.
    While(String),
    /// `repeat <n>`: fixed iteration bound for one loop level.
    Repeat(u64),
}

/// One parsed `termination_measure` directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasureDirective {
    /// `termination_measure f(pat) = expr` — function recursion measure.
    Function {
        name: String,
        pattern: String,
        expr: String,
    },
    /// `termination_measure F while …, repeat …` — loop measures.
    Loops {
        name: String,
        measures: Vec<LoopMeasure>,
    },
}

/// Parse the directive syntax of `termination_measures.sail`.
///
/// Accepts a single directive (possibly spanning lines). Returns a
/// human-readable error for malformed input instead of panicking.
pub fn parse_measure_directives(input: &str) -> Result<Vec<MeasureDirective>, String> {
    // Normalise: join continuation lines, split comma-separated loop specs later.
    let mut directives = Vec::new();
    // A new directive starts at each `termination_measure` keyword; the rest
    // (until the next keyword or EOF) belongs to it.
    let mut current = String::new();
    for line in input.lines() {
        let stripped = strip_comment(line).trim().to_string();
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with("termination_measure ") {
            if !current.trim().is_empty() {
                directives.push(parse_directive(&current)?);
            }
            current = stripped;
        } else {
            current.push(' ');
            current.push_str(&stripped);
        }
    }
    if !current.trim().is_empty() {
        directives.push(parse_directive(&current)?);
    }
    Ok(directives)
}

fn strip_comment(line: &str) -> &str {
    match line.find("/*") {
        Some(i) => &line[..i],
        None => match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        },
    }
}

fn parse_directive(text: &str) -> Result<MeasureDirective, String> {
    let rest = text
        .strip_prefix("termination_measure ")
        .ok_or_else(|| format!("expected termination_measure directive, got: {text}"))?
        .trim();
    if rest.is_empty() {
        return Err("termination_measure needs a function name".to_string());
    }
    // Function-measure form: Name(pat) = expr
    if let Some(eq) = rest.find('=') {
        let (head, expr) = (rest[..eq].trim(), rest[eq + 1..].trim());
        let (name, pattern) = split_name_pattern(head)?;
        if expr.is_empty() {
            return Err(format!("empty measure expression for {name}"));
        }
        return Ok(MeasureDirective::Function {
            name,
            pattern,
            expr: expr.to_string(),
        });
    }
    // Loop-measure form: Name [ (pat) ] while/repeat specs.
    // Note: only a paren attached to the name (no whitespace before it)
    // is a pattern; parens inside measure exprs (e.g. `pow2(n)`) are not.
    let first_ws = rest.find(char::is_whitespace);
    let paren = rest.find('(');
    let (name, rest) = match paren {
        Some(i) if first_ws.is_none_or(|w| i < w) => {
            let close = rest.find(')').ok_or_else(|| format!("unbalanced parens in: {text}"))?;
            (rest[..i].trim().to_string(), rest[close + 1..].trim().to_string())
        }
        _ => {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let name = parts.next().unwrap_or("").to_string();
            let tail = parts.next().unwrap_or("").trim().to_string();
            (name, tail)
        }
    };
    if name.is_empty() {
        return Err(format!("missing function name in: {text}"));
    }
    let mut measures = Vec::new();
    for spec in rest.split(',') {
        let spec = spec.trim();
        if spec.is_empty() {
            continue;
        }
        if let Some(expr) = spec.strip_prefix("while ") {
            let expr = expr.trim();
            if expr.is_empty() {
                return Err(format!("empty while measure for {name}"));
            }
            measures.push(LoopMeasure::While(expr.to_string()));
        } else if let Some(count) = spec.strip_prefix("repeat ") {
            let count: u64 = count
                .trim()
                .parse()
                .map_err(|_| format!("bad repeat count in: {spec}"))?;
            measures.push(LoopMeasure::Repeat(count));
        } else {
            return Err(format!("unknown loop measure spec: {spec}"));
        }
    }
    if measures.is_empty() {
        return Err(format!("no measures given for {name}"));
    }
    Ok(MeasureDirective::Loops { name, measures })
}

fn split_name_pattern(head: &str) -> Result<(String, String), String> {
    match head.find('(') {
        Some(i) => {
            let name = head[..i].trim().to_string();
            let pat = head[i..].trim().trim_start_matches('(').trim_end_matches(')').trim();
            if name.is_empty() {
                return Err(format!("missing function name in: {head}"));
            }
            Ok((name, pat.to_string()))
        }
        None => {
            let name = head.trim().to_string();
            if name.is_empty() {
                return Err(format!("missing function name in: {head}"));
            }
            Ok((name.clone(), name))
        }
    }
}

/// Minimal function-definition model for the pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunDef {
    /// Function name (single-clause; multi-clause rejection mirrors the
    /// OCaml `failwith "FD_function with more than one clause"`).
    pub name: String,
    /// Attached recursion measure, if any.
    pub measure: Option<RecMeasure>,
    /// Names of functions called by the measure whose specs must be hoisted.
    pub hoisted_specs: Vec<String>,
}

/// A measure attached by the pass, with hoisting info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedMeasure {
    pub fun_name: String,
    pub measure: RecMeasure,
    /// `val` specs moved above the definition (OCaml `moved_val_specs`).
    pub hoisted_specs: Vec<String>,
}

/// Errors of the move pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveMeasuresError {
    /// Second measure for the same function (OCaml `err_general`).
    Duplicate { name: String },
}

impl std::fmt::Display for MoveMeasuresError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveMeasuresError::Duplicate { name } => {
                write!(f, "Second termination measure given for {name}")
            }
        }
    }
}

/// The `move_termination_measures` pass (sketch).
///
/// `measures`: out-of-line `(function name, pattern, expression)` triples
/// (the `DEF_measure` nodes). `funs`: function definitions in order.
/// On success returns the updated functions plus warnings for measures
/// without a matching function.
pub struct TerminationPass;

impl TerminationPass {
    pub fn run(
        measures: Vec<(String, String, String)>,
        funs: &mut [FunDef],
        called_fns: &dyn Fn(&str) -> Vec<String>,
    ) -> Result<Vec<String>, MoveMeasuresError> {
        // 1. Collect, rejecting duplicates like the OCaml original.
        let mut table: Vec<(String, RecMeasure)> = Vec::new();
        for (name, pattern, expr) in measures {
            if table.iter().any(|(n, _)| n == &name) {
                return Err(MoveMeasuresError::Duplicate { name });
            }
            let measure = RecMeasure::new(pattern, expr, name.clone())
                .map_err(|_| MoveMeasuresError::Duplicate { name: name.clone() })?;
            table.push((name, measure));
        }
        // 2. Attach to matching fundefs (DEF_measure nodes are dropped by
        //    construction: they never enter `funs`).
        let mut warnings = Vec::new();
        let mut used = vec![false; table.len()];
        for fun in funs.iter_mut() {
            for (i, (name, measure)) in table.iter().enumerate() {
                if &fun.name == name {
                    let mut m = measure.clone();
                    m.fn_pattern = fun.name.clone();
                    fun.hoisted_specs = called_fns(&m.measure_expr);
                    fun.measure = Some(m);
                    used[i] = true;
                }
            }
        }
        // 3. Warn about unused measures.
        for (i, (name, _)) in table.iter().enumerate() {
            if !used[i] {
                warnings.push(format!("unused termination measure for function {name}"));
            }
        }
        Ok(warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_function_measure() {
        let ds = parse_measure_directives("termination_measure enabled(e) = measure(e)").unwrap();
        assert_eq!(
            ds,
            vec![MeasureDirective::Function {
                name: "enabled".to_string(),
                pattern: "e".to_string(),
                expr: "measure(e)".to_string(),
            }]
        );
    }

    #[test]
    fn parses_arm_reduce_form() {
        let ds =
            parse_measure_directives("termination_measure Reduce__1(op,input,esize,altfp) = 'N")
                .unwrap();
        assert!(matches!(ds[0], MeasureDirective::Function { .. }));
    }

    #[test]
    fn parses_while_and_repeat() {
        let ds = parse_measure_directives(
            "termination_measure ImplementedSMEVectorLength\n  while nbits, while maxbits - nbits",
        )
        .unwrap();
        assert_eq!(
            ds,
            vec![MeasureDirective::Loops {
                name: "ImplementedSMEVectorLength".to_string(),
                measures: vec![
                    LoopMeasure::While("nbits".to_string()),
                    LoopMeasure::While("maxbits - nbits".to_string()),
                ],
            }]
        );
        let ds = parse_measure_directives("termination_measure AArch64_S2Walk repeat 100, repeat 100")
            .unwrap();
        assert_eq!(
            ds,
            vec![MeasureDirective::Loops {
                name: "AArch64_S2Walk".to_string(),
                measures: vec![LoopMeasure::Repeat(100), LoopMeasure::Repeat(100)],
            }]
        );
    }

    #[test]
    fn pass_attaches_and_warns() {
        let mut funs =
            vec![FunDef { name: "enabled".to_string(), measure: None, hoisted_specs: vec![] }];
        let warnings = TerminationPass::run(
            vec![("enabled".to_string(), "e".to_string(), "measure(e)".to_string())],
            &mut funs,
            &|_| vec!["measure".to_string()],
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert!(funs[0].measure.is_some());
        assert_eq!(funs[0].hoisted_specs, vec!["measure".to_string()]);
    }

    #[test]
    fn pass_rejects_duplicates() {
        let mut funs = vec![];
        let err = TerminationPass::run(
            vec![
                ("f".to_string(), "x".to_string(), "m x".to_string()),
                ("f".to_string(), "x".to_string(), "m x".to_string()),
            ],
            &mut funs,
            &|_| vec![],
        )
        .unwrap_err();
        assert_eq!(err, MoveMeasuresError::Duplicate { name: "f".to_string() });
    }

    #[test]
    fn pass_warns_on_unused() {
        let mut funs = vec![];
        let warnings = TerminationPass::run(
            vec![("ghost".to_string(), "x".to_string(), "m x".to_string())],
            &mut funs,
            &|_| vec![],
        )
        .unwrap();
        assert_eq!(warnings.len(), 1);
    }
}
