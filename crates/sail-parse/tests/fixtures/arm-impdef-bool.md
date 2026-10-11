`arm-impdef-bool.sail` contains the original license, value signature and
`ImpDefBool` definition from `sail-arm/arm-v9.4-a/src/impdefs.sail`, revision
`aacf240a26eeb4b6b39bff81dbfc2e237b8fd683`. No function body was modified.

Its long conditional chain overflowed the recursive converter on a normal
Rust worker stack. The test supplies the model's `infixl 4 ++` declaration,
converts the full definition and serializes the resulting AST. It does not
typecheck the standalone fragment, whose referenced ARM declarations belong
to the complete model.

Fixture SHA-256:
`e45878fe9338642d146cf02994d9636aa8e6d0117075e3068b2664f3502d41c8`.
