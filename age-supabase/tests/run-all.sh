#!/usr/bin/env bash
# Run the checks in order against a running container and keep each output.
#   NAME=agesb-db ADMIN_PW_FILE=... ROBO_PW_FILE=... OUT=/private/dir ./run-all.sh
# 14 (pg_dump) needs a second container: start it with
#   NAME=agesb-db2 PORT=55441 VOLUME=agesb-data2 ../scripts/run-db.sh
#   NAME=agesb-db2 ../scripts/create-robo-role.sh
# and set WITH_DUMP=1. 15 (PostgREST) needs postgrest/postgrest:v12.2.12.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
OUT="${OUT:?directory for the outputs}"
mkdir -p "$OUT"
run() { echo "== $2"; "$here/run-sql.sh" "$1" "$here/$2" > "$OUT/${2%.sql}.out" 2>&1; echo "   ERROR lines: $(grep -c 'ERROR' "$OUT/${2%.sql}.out") (many are expected; see IMAGE-RESULTS.md)"; }

run admin 01-supabase-extensions.sql
run admin 02-other-database.sql
run robo  10-robo-cypher.sql
run robo  11-params.sql
run admin 12-labels-privs.sql
run robo  13-check-trigger.sql
if [ "${WITH_DUMP:-0}" = 1 ]; then
  echo "== 14-dump-roundtrip.sh"; WORK="$OUT/dump" "$here/14-dump-roundtrip.sh" > "$OUT/14-dump-roundtrip.out" 2>&1
fi
if [ "${WITH_POSTGREST:-0}" = 1 ]; then
  echo "== 15-postgrest-wrapper.sh"; "$here/15-postgrest-wrapper.sh" > "$OUT/15-postgrest-wrapper.out" 2>&1
fi
[ "${KEEP:-0}" = 1 ] || run robo 99-cleanup.sql
echo "outputs in $OUT"
