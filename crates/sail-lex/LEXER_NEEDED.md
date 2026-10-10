# LEXER_NEEDED.md — swap contract for `sail-lex` (Lane 16)

> Lane 61 status (MIGRATION.md #1/#2, §6.1a): **CLOSED — Option (a)**.
> `src/span.rs` deleted; `sail-ast` dependency added;
> `pub use sail_ast::{FileId, Span};`. File table still owned by a
> later (driver) lane, as before.

What Lane 17 (parser) and later lanes may assume, and what must be swapped
or finished when the workspace is merged.

## 1. Local minimal types (swap)

`src/span.rs` defines `FileId(pub u32)` and `Span { file, start, end }`
half-open byte offsets — **field-for-field identical** to
`sail-ast::{FileId, Span}` (Lane 01). `sail-lex` deliberately does **not**
depend on `sail-ast` so the lexer stays standalone.

Swap options on merge (pick one):
- (a) delete `span.rs`, add `sail-ast` as dependency, `pub use sail_ast::{FileId, Span};`
- (b) keep local types and convert at the boundary (`Span::new(f, s, e)` exists on both).

No file table lives here: `FileId` is an opaque caller-supplied handle.

## 2. Token inventory vs OCaml (`sail/src/lib/token.ml`)

All OCaml tokens exist. Renames (Rust naming):
`Function_`→`Function`, `Let_`→`Let`, `If_`→`If`, `INT/NAT/ORDER/BOOL/TYPE`→
`KindInt/KindNat/KindOrder/KindBool/KindType`, `Typedef` stays, `Op` (keyword
`operator`)→`OpKw`, `True/False/Undefined` stay.
Dropped payloads (always constant in OCaml): `Colon`, `Eq`, `EqGt`, `Unit`
are unit variants here. `MultilineString` is one joined `String`, not a
`string list`. `Num` is decimal text (arbitrary width, never parsed).

## 3. Deliberate deviations from `lexer.mll`

1. **`(* *)` comments (lane spec) with heuristic**: `(*` opens a comment
   only if followed by whitespace/newline. Rationale: real Sail code contains
   `(*TVAL)` (`aarch64/elfmain.sail:159`) which must stay `(` `*`. The OCaml
   lexer has no `(*` rule at all (`/* */` + `//` are the real syntax, both
   supported here, `/* */` nested). `(*comment*)` without spaces does NOT
   lex as a comment — documented limitation.
2. **`0X`/`0B` accepted** (OCaml: only lowercase). Superset; identical output
   on everything OCaml accepts.
3. **Operator/comment guard**: an operator never swallows `/*` or `//`
   (`+/*c*/` → `Op("+")` + comment). OCaml's longest-match could include them;
   real code always spaces operators, intent is preserved.
4. **`//` comment may end at EOF** (OCaml raises). Lenient, no error.
5. **Unterminated constructs recover**: partial token + `LexError`, never fatal.
6. **`$pragma` value**: `//` ends it, one `/* */` allowed mid-line (anything
   but blanks after it is an error, then EOL skip). Value unescaped with the
   common subset (`\n \t \r \\ \" \' \space`); exotic `Scanf.unescaped`
   sequences (`\ddd`, `\xHH`) are kept literally here.
7. **`DocBlock` content is raw inner text** (no `*`-prefix stripping like
   OCaml's `doc_comment`). Lossless; strip later if needed.
8. **`"""` opener garbage**: skip to EOL + error (OCaml would re-lex as a
   short string). Pathological input only.

## 4. Error policy (fault tolerance)

- `lex()` never fails; stream always ends with `Eof`.
- Every `LexError { span, line, col, message }` is recorded in
  `LexResult::errors`.
- Skipped input (bad chars, reserved `import`/`module`/`where`, stray `*/`)
  additionally leaves a `TokenKind::Error(msg)` placeholder in the stream so
  Lane 17 can skip tokens uniformly. Partial tokens (unterminated string /
  comment) may overlap their `Error` token spans.
- `LexResult::code_tokens()` filters `DocBlock`/`DocLine` for the parser.

## 5. Needed from other lanes (not in this crate)

- File table (`FileId` → path/contents) and includes resolution for `$include`
  (driver lane).
- Grammar using the token stream (Lane 17). Suggested entry:
  `lex(file, src)` → filter `is_doc()`/`Error` → parse.
- No spans for skipped comments are retained; add trivia collection here if a
  formatter lane needs them.
