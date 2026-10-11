#!/usr/bin/env bash
# Lane-50-Harness-SKIZZE (SMP-Boot-Abnahme).
# Status: Skelett ohne Lauf. Default ist --dry-run (nur Args + Annahmen pruefen).
# E3-Test-Moratorium: Lane-Agenten fuehren KEINE Builds/Tests aus
# (kein cargo, kein lake, kein Emulator-Start). Nur der Integrator darf
# spaeter zentral mit Memory-Limits `--run` aufrufen.
#
#   ./scripts/smp-boot-acceptance.sh --dry-run --image <Image> --dtb <f.dtb> --smp 2
#   ./scripts/smp-boot-acceptance.sh --run ...   # NUR Integrator, nach E3-Freigabe
set -euo pipefail
cd "$(dirname "$0")/.."

MODE="dry-run"
IMAGE=""; DTB=""; SMP="2"; MEM="512M"
TIMEOUT_BOOT="120"
USERSPACE_MARKER="Run /init as init process"
FREQ=""

usage() {
    sed -n '1,/^$/p' "$0"
    echo "Optionen: --run|--dry-run --image F --dtb F --smp 2|4 --mem 512M"
    echo "          --timeout-boot S --userspace-marker STR --freq HZ --check-dtb --help"
}

CHECK_DTB=0
while [ $# -gt 0 ]; do
    case "$1" in
        --run) MODE="run"; shift ;;
        --dry-run) MODE="dry-run"; shift ;;
        --image) IMAGE="${2:-}"; shift 2 ;;
        --dtb) DTB="${2:-}"; shift 2 ;;
        --smp) SMP="${2:-}"; shift 2 ;;
        --mem) MEM="${2:-}"; shift 2 ;;
        --timeout-boot) TIMEOUT_BOOT="${2:-}"; shift 2 ;;
        --userspace-marker) USERSPACE_MARKER="${2:-}"; shift 2 ;;
        --freq) FREQ="${2:-}"; shift 2 ;;
        --check-dtb) CHECK_DTB=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "FEHLER: unbekannte Option $1" >&2; usage >&2; exit 2 ;;
    esac
done

fail() { echo "FAIL: $*" >&2; exit 1; }
note() { echo "== $*"; }

# --- A7: SMP-Geometrie (N=1 zaehlt NICHT als Abnahme) ---
case "$SMP" in
    2|4) ;;
    1) fail "--smp 1 ist nur Smoke, keine Abnahme (Anti-Shortcut: Secondaries noetig)" ;;
    *) fail "--smp muss 2 oder 4 sein (gab: $SMP)" ;;
esac

[ -n "$IMAGE" ] || fail "--image <Kernel-Image> fehlt (A1)"
[ -n "$DTB" ] || fail "--dtb <file.dtb> fehlt (A2)"
[ -f "$IMAGE" ] || fail "Image nicht gefunden: $IMAGE"
[ -f "$DTB" ] || fail "DTB nicht gefunden: $DTB"

# --- A2: DTB-Knoten-Namen pruefen (reines Grep, kein Boot, kein Build) ---
check_dtb() {
    note "A2-DTB-Check (Namen only): $DTB"
    if command -v dtc >/dev/null 2>&1; then
        out="$(dtc -I dtb -O dts "$DTB" 2>/dev/null)" \
            || fail "DTB nicht lesbar (dtc-Fehler)"
        for node in 'cpus' 'psci' 'gic' 'arch_timer\|arch-timer\|timer'; do
            echo "$out" | grep -q "$node" \
                || fail "DTB enthaelt kein '$node' (A2)"
        done
        echo "$out" | grep -q 'cpu@' || fail "DTB enthaelt keine cpu@N-Knoten (A2)"
        echo "ok: DTB enthaelt cpus/psci/gic/timer/cpu@N"
    else
        note "HINWEIS: dtc fehlt — Fallback: Dateigroesse <= 2 MiB pruefen"
        size="$(wc -c < "$DTB")"
        [ "$size" -le 2097152 ] || fail "DTB > 2 MiB (A2)"
        echo "ok: DTB-Groesse $size Bytes (Inhalt prueft Integrator mit dtc)"
    fi
}

if [ "$CHECK_DTB" -eq 1 ]; then
    check_dtb
    exit 0
fi

note "Lane-50 SMP-Boot-Abnahme ($MODE): smp=$SMP mem=$MEM image=$IMAGE dtb=$DTB"
note "Erfolgskriterien: K1 Kernel-Einstieg / K2 Secondaries online / K3 Userspace / K4 keine PSCI-GIC-Fehler / K5 2x deterministisch"
note "Messpunkte (nur Integrator): M1 Wallclock t_boot/t_smp_up/t_userspace, M2 Peak-RSS vs TESTING.md, M3 Binary-Groesse, M4 Trace-Groesse"

if [ "$MODE" = "dry-run" ]; then
    check_dtb
    note "DRY-RUN OK — kein Emulator gestartet (E3). Echter Lauf nur Integrator: $0 --run --image $IMAGE --dtb $DTB --smp $SMP"
    exit 0
fi

# --- Ab hier: NUR Integrator nach E3-Freigabe. Lane-Agenten stoppen hier. ---
note "WARNUNG: echter Boot-Lauf (E3-Freigabe vorausgesetzt)"
# Platzhalter-Ablauf (Integrator verdrahtet hier die Lane-44/45-Emulator-Binary):
#   1. UART-Log + Event-Trace Pfade anlegen (perf/smp-boot-<date>.{log,trace,md})
#   2. Emulator starten: sail-emu --image "$IMAGE" --dtb "$DTB" --smp "$SMP" --mem "$MEM"
#        ${FREQ:+--freq "$FREQ"} --timeout-boot "$TIMEOUT_BOOT"  (Wallclock, M1)
#        unter /usr/bin/time -v (M2 Peak-RSS), Trace nach M4 begrenzen (50 MiB Sampling)
#   3. UART matchen: K1 'Booting Linux on physical CPU' / K2 'smp: Brought up N nodes'
#        + je Core 'Booted secondary processor' / K3 "$USERSPACE_MARKER"
#        K2 mit N-1==0 -> FAIL (kein Shortcut); Panic/oops -> FAIL (K3)
#   4. Trace matchen: 'psci: DENIED|gic: spurious-wakeup|secondary: timeout' -> FAIL (K4)
#   5. M1-M4 nach perf/smp-boot-<date>.md schreiben; 2. Wiederholungslauf fuer K5
echo "SKIZZE: Emulator-Aufruf noch nicht verdrahtet (Lane 44/45-Binary fehlt) — Abbruch ohne Lauf." >&2
exit 3
