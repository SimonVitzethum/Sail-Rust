fn main() {
    // Generate the LALRPOP tables from `src/sail.lalrpop` (E3c/E4
    // conflict fixes, see `GRAMMAR.md`). Lane 59 task (b).
    lalrpop::process_root().unwrap();
}
