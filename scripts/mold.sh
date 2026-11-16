#!/usr/bin/env bash
# The generated-SQL mold (tests/seam/README.md) — one entry point.
#
#   scripts/mold.sh check   --pg-config <PG16>/bin/pg_config --port N [options]   capture this tree, compare with the golden mold
#   scripts/mold.sh capture --pg-config … --port N --out DIR [--repeat 2] [options] capture into DIR
#   scripts/mold.sh compare GOLDEN GOT [--layer all|sql|trace] [--show N]          compare two captures
#   scripts/mold.sh bless   --pg-config … --port N [options]                       capture twice and, if the two are
#                                                                                   identical, replace tests/seam/golden
#
# check's options and exit status: tests/seam/check.sh (0 same, 1 the sql layer differs, 3 only the
# trace differs, 2 usage or a failed capture). capture's: tests/seam/capture.sh.
#
# bless rewrites the P1 gate itself. The cards K2–K9 must not (P1-CARDS.md, common rule 3: a mold
# that does not match means the code is wrong); it refuses unless OG_MOLD_BLESS=yes.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cmd="${1:-}"; [ $# -gt 0 ] && shift
case "$cmd" in
    check) exec "$ROOT/tests/seam/check.sh" "$@" ;;
    capture) exec "$ROOT/tests/seam/capture.sh" "$@" ;;
    compare) exec "${OG_PYTHON:-python3}" "$ROOT/tests/seam/compare.py" "$@" ;;
    bless) ;;
    *) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac

[ "${OG_MOLD_BLESS:-}" = yes ] || {
    echo "mold.sh bless replaces the golden mold, the gate every P1 card is held to." >&2
    echo "Only when the coordinator says so: OG_MOLD_BLESS=yes scripts/mold.sh bless …" >&2
    exit 2
}
PORT="${OG_MOLD_PGPORT:-}"; WORK="${OG_MOLD_WORK:-}"; args=(--repeat 2)
for a in "$@"; do args+=("$a"); done
while [ $# -gt 0 ]; do
    case "$1" in --port) PORT="$2"; shift 2 ;; --work) WORK="$2"; shift 2 ;; *) shift ;; esac
done
[ -n "$PORT" ] || { echo "--port N is required" >&2; exit 2; }
WORK="${WORK:-${TMPDIR:-/tmp}/og-mold-$PORT}"
has_work=0; for a in "${args[@]}"; do [ "$a" = --work ] && has_work=1; done
[ $has_work = 1 ] || args+=(--work "$WORK")

t0=$(date +%s)
[ -n "${OG_MOLD_PGPORT:-}" ] && args+=(--port "$PORT")
"$ROOT/tests/seam/capture.sh" "${args[@]}" --out "$WORK/bless" || exit 1
PGV="$(cd "$WORK/pg/bin" && ./postgres --version)"
"${OG_PYTHON:-python3}" - "$WORK/bless" "$ROOT/tests/seam/golden" "$ROOT" "$PGV" "$(( $(date +%s) - t0 ))" <<'EOF'
import lzma, os, shutil, subprocess, sys, json
src, dst, root, pgv, secs = sys.argv[1:]
if os.path.isdir(dst):
    shutil.rmtree(dst)
for d, _, fs in os.walk(src):
    for f in sorted(fs):
        p = os.path.join(d, f)
        rel = os.path.relpath(p, src)
        out = os.path.join(dst, rel)
        os.makedirs(os.path.dirname(out), exist_ok=True)
        data = open(p, "rb").read()
        if rel.startswith("trace" + os.sep):
            # the traces are large and repetitive: kept compressed (compare.py reads .xz)
            with open(out + ".xz", "wb") as w:
                w.write(lzma.compress(data, preset=9))
        else:
            with open(out, "wb") as w:
                w.write(data)
git = lambda *a: subprocess.run(["git", "-C", root, *a], capture_output=True, text=True).stdout.strip()
head = git("rev-parse", "--short", "HEAD")
tree = git("rev-parse", "--short", "HEAD:engine")
dirty = git("status", "--porcelain", "--", "engine")
summary = json.load(open(os.path.join(src, "summary.json")))
with open(os.path.join(dst, "SOURCE"), "w") as w:
    w.write(f"captured at {head}, engine/ tree {tree}{' plus uncommitted changes' if dirty else ''}, with {pgv}\n")
    w.write(f"captured twice, the two identical ({secs}s for both)\n")
    w.write(json.dumps(summary, sort_keys=True) + "\n")
print(f"golden mold written to {dst}")
EOF
