#!/usr/bin/env bash
# The mold gate: capture this tree's generated SQL and compare it with tests/seam/golden.
#
#   tests/seam/check.sh --pg-config <PG16>/bin/pg_config --port N [options]
#
# Options: everything tests/seam/capture.sh takes except --out/--repeat (--work, --engine-pkg,
# --python, --only, --keep), and
#   --layer all|sql|trace   what to compare (default all)
#   --show N                differing cases to print per file (default 3)
#   --width W               characters of a line to print (default 240)
#   --golden DIR            the mold to compare with (default tests/seam/golden)
# The capture is left in <work>/mold for a closer look (tests/seam/compare.py <golden> <work>/mold).
#
# Exit status: 0 the same; 1 the sql layer differs (og_cypher_sql / og_typeql_sql and the other
# compiler outputs); 3 only the trace layer differs (the statements the engine ran); 2 bad usage,
# or the capture itself failed. README.md says what each layer is and what to do when one differs.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GOLDEN="$ROOT/tests/seam/golden"
LAYER=all
SHOW=3
WIDTH=240
PORT="${OG_MOLD_PGPORT:-}"
WORK="${OG_MOLD_WORK:-}"
PY="${OG_PYTHON:-python3}"
pass=()
has_work=0
while [ $# -gt 0 ]; do
    case "$1" in
        --layer) LAYER="$2"; shift 2 ;;
        --show) SHOW="$2"; shift 2 ;;
        --width) WIDTH="$2"; shift 2 ;;
        --golden) GOLDEN="$2"; shift 2 ;;
        --port) PORT="$2"; pass+=("$1" "$2"); shift 2 ;;
        --work) WORK="$2"; has_work=1; pass+=("$1" "$2"); shift 2 ;;
        --python) PY="$2"; pass+=("$1" "$2"); shift 2 ;;
        --out|--repeat) echo "check.sh: $1 is not an option here (see capture.sh)" >&2; exit 2 ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
        *) pass+=("$1"); shift ;;
    esac
done
[ -n "$PORT" ] || { echo "--port N is required (the card's own PostgreSQL port)" >&2; exit 2; }
[ -d "$GOLDEN" ] || { echo "no golden mold at $GOLDEN" >&2; exit 2; }
WORK="${WORK:-${TMPDIR:-/tmp}/og-mold-$PORT}"
[ $has_work = 1 ] || pass+=(--work "$WORK")
[ -n "${OG_MOLD_PGPORT:-}" ] && pass+=(--port "$PORT")

"$ROOT/tests/seam/capture.sh" "${pass[@]}" --out "$WORK/mold" || { echo "check.sh: the capture failed" >&2; exit 2; }
echo
printf '\033[36m== compare with %s\033[0m\n' "$GOLDEN"
"$PY" "$ROOT/tests/seam/compare.py" "$GOLDEN" "$WORK/mold" --layer "$LAYER" --show "$SHOW" --width "$WIDTH"
rc=$?
case $rc in
    0) echo "mold: the same as the golden mold" ;;
    1) echo "mold: DIFFERENT — the generated SQL changed (capture in $WORK/mold)" ;;
    3) echo "mold: the generated SQL is the same; the trace DIFFERS (capture in $WORK/mold)" ;;
esac
exit $rc
