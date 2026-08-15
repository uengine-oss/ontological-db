#!/usr/bin/env bash
# SQL regression suite. Each test file is run against a fresh database; any
# error stops that file and fails the run.
set -uo pipefail

HOST="${PGHOST:-localhost}"
PORT="${PGPORT:-28816}"
DB="${OG_TEST_DB:-og_test}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

pass=0; fail=0; failed=()

for f in "$ROOT"/engine/tests/sql/*.sql; do
    name="$(basename "$f")"
    printf '%-34s' "$name"
    psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1
    psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $DB" >/dev/null 2>&1
    psql -h "$HOST" -p "$PORT" -d "$DB" -q -c "CREATE EXTENSION ontological CASCADE" >/dev/null 2>&1

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
# ---------------------------------------------------------------------------
echo
printf '%-34s' "backup round trip"
RDB="${DB}_restored"
psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $DB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$DB" -q -c "CREATE EXTENSION ontological CASCADE" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$DB" -q -f "$ROOT/examples/demo.sql" >/dev/null 2>&1
before="$(psql -h "$HOST" -p "$PORT" -d "$DB" -tAc \
  "SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)")"

pg_dump -h "$HOST" -p "$PORT" -d "$DB" -f /tmp/og_roundtrip.dump 2>/dev/null
psql -h "$HOST" -p "$PORT" -d postgres -q -c "DROP DATABASE IF EXISTS $RDB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d postgres -q -c "CREATE DATABASE $RDB" >/dev/null 2>&1
psql -h "$HOST" -p "$PORT" -d "$RDB" -q -f /tmp/og_roundtrip.dump >/dev/null 2>&1
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

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ] || { printf 'failed: %s\n' "${failed[*]}"; exit 1; }
