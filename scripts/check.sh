#!/usr/bin/env bash
# Lane-15-Check: fmt + clippy + unsafe-Verbot + optional cargo-geiger.
# Aufruf: ./scripts/check.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== cargo fmt --check =="
cargo fmt --all -- --check

echo "== cargo clippy -D warnings =="
cargo clippy --workspace --all-targets -- -D warnings

echo "== forbid(unsafe_code) in jedem Crate =="
missing=0
for lib in crates/*/src/lib.rs crates/*/src/main.rs; do
    [ -f "$lib" ] || continue
    if ! grep -q '#!\[forbid(unsafe_code)\]' "$lib"; then
        echo "FEHLT: $lib hat kein #![forbid(unsafe_code)]"
        missing=1
    fi
done
[ "$missing" -eq 0 ] || exit 1
echo "ok: alle Crate-Roots haben forbid(unsafe_code)"

echo "== direkter unsafe-Gebrauch (muss leer sein) =="
# Sucht das Token 'unsafe' ausserhalb der forbid-Zeile.
if grep -rn --include='*.rs' -E '(^|[^_a-zA-Z])unsafe([^_a-zA-Z]|$)' crates/ | grep -v 'forbid(unsafe_code)'; then
    echo "FEHLER: unsafe-Code gefunden (siehe oben)"
    exit 1
fi
echo "ok: kein unsafe-Code in crates/"

echo "== cargo geiger (falls installiert) =="
if command -v cargo-geiger >/dev/null 2>&1; then
    cargo geiger --workspace --all-targets --forbid-only
else
    echo "HINWEIS: cargo-geiger nicht installiert — Sprung."
    echo "Installieren: cargo install cargo-geiger"
    echo "CI laesst geiger trotzdem laufen (siehe .github/workflows/ci.yml)."
fi

echo "ALLE CHECKS OK"
