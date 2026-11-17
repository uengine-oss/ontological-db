#!/bin/sh
# run-sql.sh <robo|admin> <file.sql> [db]
# Runs a SQL file inside the container through a pty, so statements, results
# and errors come out in order (psql buffers stdout when it is not a tty).
# robo connects to the container's own network address, so it goes through
# scram-sha-256 like an outside client; loopback is `trust` in the image.
#   NAME=agesb-db ADMIN_PW_FILE=... ROBO_PW_FILE=... ./run-sql.sh robo 10-robo-cypher.sql
set -eu
NAME="${NAME:-agesb-db}"
who="$1"; f="$2"; db="${3:-postgres}"
case "$who" in
  robo)  user=robo; pw="$(cat "${ROBO_PW_FILE:?}")"; host='$(hostname -i | cut -d" " -f1)';;
  admin) user=supabase_admin; pw="$(cat "${ADMIN_PW_FILE:?}")"; host=127.0.0.1;;
  *) echo "usage: $0 <robo|admin> <file.sql> [db]" >&2; exit 2;;
esac
docker exec "$NAME" mkdir -p /tmp/agesb-tests
docker cp -q "$f" "$NAME:/tmp/agesb-tests/run.sql"
docker exec -e PGPASSWORD="$pw" "$NAME" sh -c \
  "script -qec \"psql -X -e -v ON_ERROR_STOP=0 -P pager=off -U $user -h $host -d $db -f /tmp/agesb-tests/run.sql\" /dev/null" | tr -d '\r'
docker exec "$NAME" rm -rf /tmp/agesb-tests
