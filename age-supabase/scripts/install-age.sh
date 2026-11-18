#!/usr/bin/env bash
# Install AGE into an already initialised Supabase data volume (fresh volumes get
# it from the image's initdb migration). Idempotent; runs as supabase_admin.
# Same SQL as the image: the extension in `postgres`, in every other database and
# template1 (AGE 1.8.0's TRUNCATE hook), and the graph database AGE_GRAPH_DB
# (default `graph`, LC_COLLATE C; empty = do not make one).
#
#   NAME=supabase-db ADMIN_PW_FILE=... ./scripts/install-age.sh
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${NAME:-agesb-db}"
DB="${DB:-postgres}"
AGE_GRAPH_DB="${AGE_GRAPH_DB-graph}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:?path of a file holding the supabase_admin password}"

docker exec -i -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" -e AGE_GRAPH_DB="$AGE_GRAPH_DB" "$NAME" \
  psql -X -q -v ON_ERROR_STOP=1 -U supabase_admin -h 127.0.0.1 -d "$DB" \
  < "$here/sql/99999999999999_age.sql"

docker exec -i -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" "$NAME" \
  psql -X -At -U supabase_admin -h 127.0.0.1 -d "$DB" \
  -c "select 'age ' || extversion || ' in ' || extnamespace::regnamespace from pg_extension where extname = 'age'" \
  -c "show shared_preload_libraries"
# every database that takes connections should now have the extension
docker exec -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" "$NAME" sh -c '
  for d in $(psql -X -At -U supabase_admin -h 127.0.0.1 -d postgres -c "select datname from pg_database where datallowconn order by 1"); do
    psql -X -At -U supabase_admin -h 127.0.0.1 -d "$d" -c "select current_database() || '"'"': collate '"'"' || (select datcollate from pg_database where datname = current_database()) || '"'"', age '"'"' || coalesce((select extversion from pg_extension where extname = '"'"'age'"'"'), '"'"'MISSING'"'"')"
  done' 
