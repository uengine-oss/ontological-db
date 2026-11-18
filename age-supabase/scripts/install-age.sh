#!/usr/bin/env bash
# Install AGE into an already initialised Supabase data volume (fresh volumes get
# it from the image's initdb migration). Idempotent; runs as supabase_admin.
#
#   NAME=supabase-db ADMIN_PW_FILE=... ./scripts/install-age.sh
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
NAME="${NAME:-agesb-db}"
DB="${DB:-postgres}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:?path of a file holding the supabase_admin password}"

docker exec -i -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" "$NAME" \
  psql -X -q -v ON_ERROR_STOP=1 -U supabase_admin -h 127.0.0.1 -d "$DB" \
  < "$here/sql/99999999999999_age.sql"

docker exec -i -e PGPASSWORD="$(cat "$ADMIN_PW_FILE")" "$NAME" \
  psql -X -At -U supabase_admin -h 127.0.0.1 -d "$DB" \
  -c "select 'age ' || extversion || ' in ' || extnamespace::regnamespace from pg_extension where extname = 'age'" \
  -c "show shared_preload_libraries"
