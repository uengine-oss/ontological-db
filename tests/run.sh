#!/usr/bin/env bash
# SQL regression suite. Each test file is run against a fresh database; any
# error stops that file and fails the run.
#
# The files are tests/common/sql (no storage layout in them: they run on every
# edition) and tests/<edition>/sql (they read that edition's own tables), run
# together in file-name order. See tests/README.md for which is which.
#
# Env: PGHOST PGPORT PGUSER, OG_TEST_DB (og_test), OG_EDITION (v1 | age, default v1),
#      OG_EXTENSION (default ontological for v1, ontological_age for age),
#      OG_DUMP_FILE (the backup round trip's dump, default /tmp/og_roundtrip.dump).
set -uo pipefail

HOST="${PGHOST:-localhost}"
PORT="${PGPORT:-28816}"
DB="${OG_TEST_DB:-og_test}"
EDITION="${OG_EDITION:-v1}"
case "$EDITION" in
    v1)  EXT="${OG_EXTENSION:-ontological}" ;;
    age) EXT="${OG_EXTENSION:-ontological_age}" ;;
    *)   echo "OG_EDITION must be v1 or age, not '$EDITION'" >&2; exit 2 ;;
esac
DUMP="${OG_DUMP_FILE:-/tmp/og_roundtrip.dump}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

pass=0; fail=0; failed=()

files=()
while IFS= read -r f; do files+=("$f"); done < <(
    for d in "$HERE/common/sql" "$HERE/$EDITION/sql"; do
        [ -d "$d" ] && find "$d" -maxdepth 1 -name '*.sql'
    done | awk -F/ '{print $NF "\t" $0}' | sort | cut -f2-)

for f in ${files[@]+"${files[@]}"}; do
    name="$(basename "$f")"
    printf '%-34s' "$name"
    psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1
    psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $DB" >/dev/null 2>&1
    psql -h "$HOST" -p "$PORT" -d "$DB" -q -c "CREATE EXTENSION $EXT CASCADE" >/dev/null 2>&1

    out="$(psql -h "$HOST" -p "$PORT" -d "$DB" -q -f "$f" 2>&1)"
    # Files that deliberately end in an expected error mark it with EXPECT_ERROR.
    expected="$(grep -c 'EXPECT_ERROR' "$f" || true)"
    actual="$(printf '%s' "$out" | grep -c '^ERROR\|^psql.*ERROR' || true)"

    if [ "$actual" -le "$expected" ]; then
        echo "ok"
        pass=$((pass+1))
    else
        echo "FAIL"
        printf '%s\n' "$out" | grep -A2 'ERROR' | head -12 | sed 's/^/    /'
        fail=$((fail+1)); failed+=("$name")
    fi
done

# ---------------------------------------------------------------------------
# Backup round trip. Extension-owned tables are invisible to pg_dump unless they
# are registered with pg_extension_config_dump(), and getting that wrong restores
# an empty graph in silence — so it gets its own gate.
#
# v1 only: it counts through v1's og_node_view / og_edge_view and checks v1's
# og_check_integrity(). The age edition needs its own (tests/README.md).
# ---------------------------------------------------------------------------
if [ "$EDITION" = v1 ]; then
echo
printf '%-34s' "backup round trip"
RDB="${DB}_restored"
psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $DB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$DB" -q -c "CREATE EXTENSION $EXT CASCADE" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$DB" -q -f "$ROOT/examples/demo.sql" >/dev/null 2>&1
before="$(psql -h "$HOST" -p "$PORT" -d "$DB" -tAc \
  "SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)")"

pg_dump -h "$HOST" -p "$PORT" -d "$DB" -f "$DUMP" 2>/dev/null
psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $RDB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $RDB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$RDB" -q -f "$DUMP" >/dev/null 2>&1
after="$(psql -h "$HOST" -p "$PORT" -d "$RDB" -tAc \
  "SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)" 2>/dev/null)"
query_ok="$(psql -h "$HOST" -p "$PORT" -d "$RDB" -tAc \
  "SELECT og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')" 2>/dev/null)"

if [ "$before" = "$after" ] && [ -n "$before" ] && [ "$before" != "0/0" ] && [ -n "$query_ok" ]; then
    echo "ok ($before nodes/edges preserved)"
    pass=$((pass+1))
else
    echo "FAIL (before=$before after=$after query=$query_ok)"
    fail=$((fail+1)); failed+=("backup round trip")
fi
psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $RDB" >/dev/null 2>&1

echo
echo "-- structural integrity --"
psql -h "$HOST" -p "$PORT" -d "$DB" -tAc "SELECT count(*) FROM og_check_integrity()" 2>/dev/null \
  | while read -r n; do
      if [ "${n:-0}" = "0" ]; then echo "integrity                          ok"
      else echo "integrity                          FAIL ($n violations)"; fi
    done
fi

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ] || { printf 'failed: %s\n' "${failed[*]}"; exit 1; }
