fn main() {
    // The LALRPOP grammar in `src/sail.lalrpop` is kept as a documentation
    // artifact (see `GRAMMAR.md`): the direct Menhir port has LALR(1)
    // conflicts that LALRPOP rejects (Menhir resolves them as shift).
    // This crate therefore builds as a skeleton (AST + lexer + coverage
    // tests) without invoking the LALRPOP code generator.
}
