# Legacy Sail library compatibility

`trace.sail` is the unmodified upstream Sail **0.17.1** file required by the
ARMv9.4-A prelude. It defines `enable_tracing`, `disable_tracing` and `is_tracing`,
including their real Sail fallback bodies; it is not an empty include stub.
The newer 0.20.3 library inventory remains pinned separately in `sail-lib/`.

Source: https://github.com/rems-project/sail/blob/0.17.1/lib/trace.sail

SHA-256: `7995b8ff7744e900e72cac1bf1a4a9bf0c065a88d885e96507dcf7ca5b677544`

The original BSD license and copyright notice are retained in the file.
An explicitly configured `SAIL_RS_LIBDIR` takes precedence; missing files in
that external directory are errors, rather than falling back silently.
