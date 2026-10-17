#![forbid(unsafe_code)]
//! Identifier and literal spelling (`fix_id`, `doc_id_ctor`, `doc_lit`, ...).

use sail_ast::{Interner, Lit, LitKind, Name, SimpleInterner};

use crate::ctx::Ctx;
use crate::doc::{string, utf8string, Doc};

/// `Util.zencode_string` for operator names.
#[must_use]
pub fn zencode(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(c),
            '_' => out.push_str("_"),
            ' ' => out.push_str("zspace"),
            c => {
                // The OCaml encoder spells operator characters out.
                let name = match c {
                    '+' => "zp",
                    '-' => "zm",
                    '*' => "zt",
                    '/' => "zs",
                    '<' => "zl",
                    '>' => "zg",
                    '=' => "ze",
                    '!' => "zn",
                    '&' => "za",
                    '|' => "zb",
                    '^' => "zc",
                    '@' => "zat",
                    '%' => "zpc",
                    '.' => "zd",
                    ':' => "zc",
                    '~' => "zt",
                    '?' => "zq",
                    '$' => "zdl",
                    '#' => "zh",
                    ',' => "zcm",
                    _ => "zx",
                };
                out.push_str(name);
            }
        }
    }
    out
}

/// `fix_id`.
#[must_use]
pub fn fix_id(name: &str) -> String {
    match name {
        "_lean_wildcard" => "_".to_owned(),
        "rec" | "def" | "at" | "alias" | "break" | "meta" | "class" => format!("{name}'"),
        "main" => "sail_main".to_owned(),
        "?" => "questionMark".to_owned(),
        _ if name.contains('#') => fix_id(&name.split('#').collect::<Vec<_>>().join("_")),
        _ => name.to_owned(),
    }
}

/// Name text of an AST name under its own interner.
#[must_use]
pub fn name_text(name: &Name, it: &SimpleInterner) -> String {
    Interner::resolve(it, name.symbol).unwrap_or("?").to_owned()
}

/// Spelling of an identifier text (`doc_id_ctor` without constructor renames).
#[must_use]
pub fn id_text(text: &str, is_operator: bool) -> String {
    if text == "and_bool" {
        return "and_bool".to_owned();
    }
    if text == "or_bool" {
        return "or_bool".to_owned();
    }
    if is_operator {
        return zencode(&format!("op {text}"));
    }
    fix_id(text)
}

/// `doc_id_ctor ?ren_ctor ctx id`.
#[must_use]
pub fn doc_id_ctor(ctx: &Ctx<'_>, it: &SimpleInterner, id: &Name, ren_ctor: bool) -> Doc {
    let text = name_text(id, it);
    if ren_ctor && ctx.global.oracle.is_union_ctor(&text) {
        if let Some(prefix) = ctx.global.oracle.ctor_rename(&text) {
            return string(&format!("{prefix}.{text}"));
        }
    }
    string(&id_text(&text, id.is_operator))
}

/// `doc_id_ctor` for a plain text identifier (constructor, field, kid).
#[must_use]
pub fn doc_text(text: &str) -> Doc {
    string(&fix_id(text))
}

/// `doc_id_typ_ctor`: qualify a type name shadowed by a value.
#[must_use]
pub fn doc_id_typ_ctor(ctx: &Ctx<'_>, text: &str) -> Doc {
    if ctx.global.oracle.value_named(text) {
        string(&format!("{}.{}", ctx.global.out_name_camel, fix_id(text)))
    } else {
        string(&fix_id(text))
    }
}

/// `doc_kid`: `k_n` unless renamed to a function argument.
#[must_use]
pub fn doc_kid(ctx: &Ctx<'_>, kid: &str) -> Doc {
    if let Some(Some(id)) = ctx.kid_id_renames.get(kid) {
        return doc_text(id);
    }
    let bare = kid.strip_prefix('\'').unwrap_or(kid);
    string(&fix_id(&format!("k_{bare}")))
}

pub fn doc_big_int(text: &str) -> Doc {
    if text.starts_with('-') {
        string(text).parens()
    } else {
        string(text)
    }
}

fn lean_escape_string(s: &str) -> String {
    s.replace('"', "\\\"")
}

/// `doc_lit ~width`.
#[must_use]
pub fn doc_lit(it: &SimpleInterner, lit: &Lit, width: bool) -> Doc {
    let text = |s: sail_ast::Symbol| Interner::resolve(it, s).unwrap_or("").to_owned();
    match &lit.kind {
        LitKind::Unit => string("()"),
        LitKind::False => string("false"),
        LitKind::True => string("true"),
        LitKind::Num(n) => doc_big_int(&text(*n)),
        LitKind::Hex(h) => {
            let digits = text(*h);
            if digits.is_empty() {
                return string("BitVec.nil");
            }
            let spec = if width { format!("#{}", digits.len() * 4) } else { String::new() };
            utf8string(&format!("0x{}{}", digits.to_uppercase(), spec))
        }
        LitKind::Bin(b) => {
            let digits = text(*b);
            if digits.is_empty() {
                return string("BitVec.nil");
            }
            let spec = if width { format!("#{}", digits.len()) } else { String::new() };
            if digits == "0" || digits == "1" {
                return string(&format!("{digits}{spec}"));
            }
            utf8string(&format!("0b{digits}{spec}"))
        }
        LitKind::String(s) => utf8string(&format!("\"{}\"", lean_escape_string(&text(*s)))),
        LitKind::Real(r) => utf8string(&text(*r)),
    }
}
