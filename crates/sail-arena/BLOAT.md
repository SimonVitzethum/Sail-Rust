# BLOAT.md — Hinweise zur Binärgröße (`cargo bloat`, Lane 37)

`cargo-bloat` ist in dieser Umgebung **nicht installiert**; diese Datei ist
daher Anleitung + Design-Doku, kein Messprotokoll (Spec: „als Doku“).

## Messen (wenn `cargo-bloat` verfügbar ist)

```sh
cargo install cargo-bloat
# Größte Symbole im Fixture-Binary (dominante Monomorphisierungen finden):
cargo bloat --release --example peak -n 30
# Kisten-Anteile (wer zieht was rein?):
cargo bloat --release --example peak --crates
```

Ohne `cargo-bloat` als grober Ersatz (nur Symbolgrößen, kein Demangling):

```sh
cargo build --release --example peak
nm --print-size -S --size-sort target/release/examples/peak \
  | c++filt | grep -i -E "arena|genarena|shard" | tail -20
```

## Worauf achten

1. **Monomorphisierung pro `T`:** `ShardArena<T>` und `GenArena<T>`
   werden pro Nutzdaten-Typ instanziiert. Die Policy (`sizing`,
   `live_bytes_estimate(usize, usize)`) ist bewusst **nicht-generisch**,
   damit genau eine Kopie im Binary landet. Neue generische Helfer nur
   mit Begründung hinzufügen.
2. **T-Inventar klein halten:** Lieber wenige, wiederverwendete
   Knoten-Typen in Arenen legen als pro Lane einen eigenen. Jede neue
   `ShardArena<T>`-Instanz kostet `.text` + Debug-Info.
3. **Abhängigkeiten:** `sail-arena` hängt nur an `sail-ast` (kein `serde`,
   kein `rand`, keine Allocator-Kisten). `--crates`-Output muss das
   bestätigen; jede neue Dependency hier ist bloat-relevant und gehört
   in die Commit-Message begründet.
4. **Release-Profil:** Größenvergleich immer mit `--release`
   (Debug-Symbole verfälschen alles). Für echte Auslieferung zusätzlich
   `strip` / `split-debuginfo` prüfen (Release-Packaging: Lane 40).

## Design-Entscheidungen dieser Lane (bloat-arm)

- Kein eigener Slot-/Freilisten-Code (wäre Duplikat von `GenArena` und
  neue generische Flächen): reiner Wrapper, keine Logik-Duplikate.
- `Handle { shard: u32, id: NodeId }`: `Copy`, keine Lifetimes, keine
  Trait-Objekte, kein dynamischer Dispatch.
- `peak_rss_kib`: reines std-IO (`/proc/self/status`), kein `libc`-Dep.
