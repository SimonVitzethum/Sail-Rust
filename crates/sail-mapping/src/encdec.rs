//! Encdec/execute split sketch.
//!
//! The Lean backend runs `("split", [String_arg "execute"])`
//! (`sail_plugin_lean.ml`): the `execute` function over the instruction union
//! is split per constructor into `execute_<Ctor>` helpers. The realized
//! mapping functions (`*_forwards`, `*_backwards`, `*_matches`) form the
//! complementary shard: they are the pure encode/decode tables (`encdec`)
//! that the decoder and the encoder both call.
//!
//! This module is deliberately a *sketch* (classification + partition over
//! definition names): the actual AST splitting of `execute` clauses
//! (`rewrite_split_fun_ctor_pats`) belongs to the function-definition lane.
//! What this lane owns is the mapping side — which definitions are encdec
//! tables and therefore must not be pulled into the `execute` shard.

#![forbid(unsafe_code)]

/// Which output shard a definition belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shard {
    /// Encode/decode tables from realized mappings.
    Encdec,
    /// Everything else (including the split `execute_*` helpers).
    Execute,
}

/// Classify a realized mapping function name as [`Shard::Encdec`].
///
/// A name is an encdec table if it ends with one of the suffixes produced by
/// `realize_mappings`: `_forwards`, `_backwards`, `_forwards_matches`,
/// `_backwards_matches`.
pub fn is_encdec_table(name: &str) -> bool {
    ["_forwards_matches", "_backwards_matches", "_forwards", "_backwards"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// Partition definition names into `(encdec, execute)` shards.
///
/// `mapping_names` are the source mapping names (e.g. `size_bits`); any
/// definition classified by [`is_encdec_table`] lands in `encdec`, the rest
/// in `execute`. Order is preserved.
pub fn split_encdec_execute<'a>(
    def_names: impl IntoIterator<Item = &'a str>,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut encdec = Vec::new();
    let mut execute = Vec::new();
    for name in def_names {
        if is_encdec_table(name) {
            encdec.push(name);
        } else {
            execute.push(name);
        }
    }
    (encdec, execute)
}

/// Classify a single definition name.
pub fn classify_def(name: &str) -> Shard {
    if is_encdec_table(name) {
        Shard::Encdec
    } else {
        Shard::Execute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shard layout for the `mapping.sail` fixture: every realized mapping
    /// function is an encdec table; user helpers (`hex_bits_forwards`, ...)
    /// and `execute` stay in the execute shard.
    #[test]
    fn fixture_defs_split_into_encdec_and_execute() {
        let defs = [
            "size_bits_forwards",
            "size_bits_backwards",
            "size_bits_forwards_matches",
            "size_bits_backwards_matches",
            "ta_flag_forwards",
            "hex_bits_forwards",
            "hex_bits_backwards_matches",
            "hex_bits_2_forwards",
            "vtype_assembly_forwards",
            "vtype_assembly_backwards_matches",
            "execute",
            "initialize_registers",
        ];
        let (encdec, execute) = split_encdec_execute(defs);
        assert_eq!(
            encdec,
            [
                "size_bits_forwards",
                "size_bits_backwards",
                "size_bits_forwards_matches",
                "size_bits_backwards_matches",
                "ta_flag_forwards",
                "hex_bits_forwards",
                "hex_bits_backwards_matches",
                "hex_bits_2_forwards",
                "vtype_assembly_forwards",
                "vtype_assembly_backwards_matches",
            ]
        );
        assert_eq!(execute, ["execute", "initialize_registers"]);
    }

    #[test]
    fn suffix_classification() {
        assert_eq!(classify_def("size_bits_forwards"), Shard::Encdec);
        assert_eq!(classify_def("x_backwards_matches"), Shard::Encdec);
        assert_eq!(classify_def("execute_ADD"), Shard::Execute);
        assert_eq!(classify_def("forwards"), Shard::Execute);
        assert_eq!(classify_def("size_bits"), Shard::Execute);
    }
}
