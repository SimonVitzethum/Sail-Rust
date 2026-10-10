#!/usr/bin/env python3
"""rig-triage.py — Rig-Log lesen, Errors clustern, Issue-Liste zeigen.

Lane 117 (rig-triage). E3: liest nur Logs, startet keine Last.

Aufruf:
    ./scripts/rig-triage.py [--log DATEI] [--out TRIAGE.md] [--top N] [--no-write]
    ./scripts/rig-triage.py --log logs/limited-run-xyz.log --no-write   # nur stdout

Default: neuestes logs/limited-run-*.log nach mtime; schreibt TRIAGE.md
im Repo-Root UND druckt die Issue-Liste nach stdout (Done-Kriterium:
"Ein Befehl zeigt Issue-Liste des letzten Laufs").

Erkannte Fehlerarten (Kind):
    rustc-error   Zeilen mit error[EXXXX]/`error:` + folgende `-->`-Orte
    rustc-warning-cap  `limited-run: N Warning-Zeilen unterdrueckt`-Summary
    test-failure  `test <name> ... FAILED`, `failures:`-Bloecke
    panic         `panicked at <datei>:<zeile>` / `thread '...' panicked`
    rig-exit      `limited-run: exit=N` (N != 0)
    generic-error sonstige Zeilen mit error/FAILED/failure (case-insensitiv)

Cluster-Schluessel: (Art, Datei:Zeile, Meldungsnorm). Owner-Lane: aus
`crates/<name>/` im Pfad via CRATE_LANES (Quelle: docs/ARCHITECTURE.md §2);
ohne Crate-Bezug: `-`.
"""
import argparse
import os
import re
import sys
from collections import Counter

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOG_DIR = os.path.join(REPO, "logs")

# Crate -> Owner-Lane (docs/ARCHITECTURE.md §2, Stand Lane 53).
CRATE_LANES = {
    "sail-ast": "01", "sail-parse": "02", "sail-resolve": "03",
    "sail-nexp": "04", "sail-ty": "05", "sail-solver": "06",
    "sail-rewrite": "07", "sail-effects": "08", "sail-lower": "09",
    "sail-lean": "10", "sail-shard": "11", "sail-lake": "12",
    "sail-oracle": "13", "sail-perf": "14", "sail-lex": "16",
    "sail-grammar": "17", "sail-prelude": "18", "sail-regs": "19",
    "sail-mapping": "20", "sail-patterns": "21", "sail-bitvec": "22",
    "sail-int": "23", "sail-real": "24", "sail-extern": "25",
    "sail-termination": "26", "sail-loops": "27", "sail-except": "28",
    "sail-archsem": "29", "sail-sysregs-shard": "31", "sail-sve-sme": "32",
    "sail-instrs-core": "33", "sail-exe": "34", "sail-diag": "35",
    "sail-cache": "36", "sail-cli": "38",
}

RE_ERROR = re.compile(r"^error(\[E\d+\])?:\s*(.*)")
RE_LOC = re.compile(r"^\s*-->\s*(\S+?):(\d+)(?::(\d+))?")
RE_PANIC = re.compile(r"panicked at (\S+?):(\d+)(?::(\d+))?")
RE_TEST_FAIL = re.compile(r"^test\s+(\S+)\s+\.\.\.\s+FAILED")
RE_GENERIC = re.compile(r"(?i)\b(error|failed|failure|panicked)\b")
RE_CRATE = re.compile(r"crates/([a-z0-9-]+)/")
RE_EXIT = re.compile(r"limited-run:\s*exit=(\d+)")
RE_WARNCAP = re.compile(r"limited-run:\s*(\d+)\s+Warning-Zeilen unterdrueckt")
RE_FILELINE = re.compile(r"([A-Za-z0-9_@.+/\\-]+?\.(?:rs|sail|toml|sh|py|md|lalrpop)):(\d+)")


def owner_of(path):
    m = RE_CRATE.search(path or "")
    if m:
        c = m.group(1)
        lane = CRATE_LANES.get(c)
        return f"Lane {lane} ({c})" if lane else f"- ({c})"
    return "-"


def norm_msg(msg):
    msg = re.sub(r"0x[0-9a-fA-F]+", "0x…", msg)
    msg = re.sub(r"\b\d{3,}\b", "N", msg)
    return msg.strip()[:160]


def newest_log(log_dir=LOG_DIR):
    cands = []
    if os.path.isdir(log_dir):
        for name in os.listdir(log_dir):
            if name.startswith("limited-run-") and name.endswith(".log"):
                p = os.path.join(log_dir, name)
                try:
                    cands.append((os.path.getmtime(p), p))
                except OSError:
                    pass
    if not cands:
        return None
    return sorted(cands)[-1][1]


def triage(lines):
    """lines -> (issues, meta). issues: Liste von Dicts
    {kind, loc, owner, count, sample}."""
    counts = Counter()
    samples = {}
    exit_code = None
    warn_suppressed = 0
    total = 0
    pending_error = None  # letzte error:-Meldung, wartet auf -->-Ort

    for raw in lines:
        total += 1
        line = raw.rstrip("\n")
        s = line.strip()

        m = RE_EXIT.search(s)
        if m:
            exit_code = int(m.group(1))
            continue
        m = RE_WARNCAP.search(s)
        if m:
            warn_suppressed += int(m.group(1))
            continue

        m = RE_ERROR.match(s)
        if m:
            pending_error = norm_msg(m.group(2) or "error")
            key = ("rustc-error", "-", pending_error)
            counts[key] += 1
            samples.setdefault(key, line.strip()[:200])
            continue

        m = RE_LOC.match(s)
        if m and pending_error:
            path, lno = m.group(1), m.group(2)
            loc = f"{path}:{lno}"
            key = ("rustc-error", loc, pending_error)
            counts[key] += 1
            samples.setdefault(key, line.strip()[:200])
            # die ortlose Vorab-Zaehlung wieder abziehen
            bare = ("rustc-error", "-", pending_error)
            counts[bare] -= 1
            if counts[bare] <= 0:
                del counts[bare]
                samples.pop(bare, None)
            pending_error = None
            continue

        m = RE_PANIC.search(s)
        if m:
            loc = f"{m.group(1)}:{m.group(2)}"
            msg = norm_msg(s)
            key = ("panic", loc, msg)
            counts[key] += 1
            samples.setdefault(key, line.strip()[:200])
            continue

        m = RE_TEST_FAIL.match(s)
        if m:
            key = ("test-failure", "-", norm_msg("test " + m.group(1)))
            counts[key] += 1
            samples.setdefault(key, line.strip()[:200])
            continue

        if RE_GENERIC.search(s) and not s.startswith("limited-run:"):
            m2 = RE_FILELINE.search(s)
            loc = f"{m2.group(1)}:{m2.group(2)}" if m2 else "-"
            key = ("generic-error", loc, norm_msg(s))
            counts[key] += 1
            samples.setdefault(key, line.strip()[:200])
            continue

    issues = []
    for (kind, loc, msg), n in counts.most_common():
        issues.append({
            "kind": kind, "loc": loc, "owner": owner_of(loc + " " + msg),
            "count": n, "sample": samples[(kind, loc, msg)],
        })
    # Tests/Panics ohne Dateiort: Owner aus Sample-Pfad nachziehen.
    for it in issues:
        if it["owner"] == "-":
            it["owner"] = owner_of(it["sample"])
    if exit_code not in (None, 0):
        issues.append({"kind": "rig-exit", "loc": "-",
                       "owner": "-", "count": 1,
                       "sample": f"limited-run: exit={exit_code}"})
    meta = {"lines": total, "exit": exit_code,
            "warn_suppressed": warn_suppressed}
    return issues, meta


def render(log_path, issues, meta, top):
    out = []
    out.append(f"# Rig-Triage — `{os.path.basename(log_path)}`")
    out.append("")
    out.append(f"Log: `{log_path}` — {meta['lines']} Zeilen, "
               f"exit={meta['exit']}, "
               f"{meta['warn_suppressed']} Warning-Zeilen unterdrueckt.")
    out.append("")
    if not issues:
        out.append("Keine Errors gefunden.")
        return "\n".join(out) + "\n"
    out.append("| # | Art | Datei:Zeile | Owner | n | Meldung |")
    out.append("|---|---|---|---|---|---|")
    for i, it in enumerate(issues[:top], 1):
        msg = it["sample"].replace("|", "/")
        out.append(f"| {i} | {it['kind']} | {it['loc']} | {it['owner']} "
                   f"| {it['count']} | {msg} |")
    if len(issues) > top:
        out.append(f"\n… +{len(issues) - top} weitere Cluster (Top {top} gezeigt).")
    return "\n".join(out) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description="Rig-Log triagieren (read-only).")
    ap.add_argument("--log", default=None, help="Logdatei (Default: neuestes logs/limited-run-*.log)")
    ap.add_argument("--out", default="TRIAGE.md", help="Zieldatei (Default: TRIAGE.md im Repo-Root)")
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--no-write", action="store_true", help="nur stdout, keine Datei schreiben")
    args = ap.parse_args(argv)

    log_path = args.log or newest_log()
    if not log_path or not os.path.isfile(log_path):
        print("FEHLER: kein Rig-Log gefunden (logs/limited-run-*.log). "
              "Erst einen Rig-Lauf starten, dann triagieren.", file=sys.stderr)
        return 2
    with open(log_path, encoding="utf-8", errors="replace") as f:
        issues, meta = triage(f)
    text = render(log_path, issues, meta, args.top)
    print(text, end="")
    if not args.no_write:
        out = args.out if os.path.isabs(args.out) else os.path.join(REPO, args.out)
        with open(out, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"(TRIAGE geschrieben: {out})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
