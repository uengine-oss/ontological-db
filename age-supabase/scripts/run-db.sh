#!/usr/bin/env bash
# Start a Supabase Postgres + AGE container for local testing.
#
#   SECRETS_DIR=/path/to/private/dir ./scripts/run-db.sh
#
# The supabase_admin / postgres password is generated on first start and kept in
# $SECRETS_DIR/admin.password (mode 0600). The container listens on 127.0.0.1 only.
set -euo pipefail

IMAGE="${IMAGE:-uengine/supabase-postgres-age:15.8.1.060-age1.8.0}"
NAME="${NAME:-agesb-db}"
PORT="${PORT:-55439}"
VOLUME="${VOLUME:-${NAME/-db/-data}}"
NETWORK="${NETWORK:-agesb-net}"
MEMORY="${MEMORY:-1500m}"
CPUS="${CPUS:-2}"
SECRETS_DIR="${SECRETS_DIR:?set SECRETS_DIR to a private directory for the generated passwords}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:-$SECRETS_DIR/admin.password}"

umask 077
mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"
if [ ! -s "$ADMIN_PW_FILE" ]; then
  python3 -c 'import secrets; print(secrets.token_urlsafe(24))' > "$ADMIN_PW_FILE"
fi
chmod 600 "$ADMIN_PW_FILE"

docker network inspect "$NETWORK" >/dev/null 2>&1 || docker network create "$NETWORK" >/dev/null
docker volume inspect "$VOLUME" >/dev/null 2>&1 || docker volume create "$VOLUME" >/dev/null

# the password goes in through an env file so it does not show up in `ps`
envfile="$(mktemp "$SECRETS_DIR/.env.XXXXXX")"
trap 'rm -f "$envfile"' EXIT
printf 'POSTGRES_PASSWORD=%s\n' "$(cat "$ADMIN_PW_FILE")" > "$envfile"

docker run -d --name "$NAME" --network "$NETWORK" \
  --memory "$MEMORY" --cpus "$CPUS" --restart unless-stopped \
  -p "127.0.0.1:${PORT}:5432" \
  --env-file "$envfile" \
  -v "$VOLUME:/var/lib/postgresql/data" \
  "$IMAGE" >/dev/null

printf 'waiting for %s ' "$NAME"
for _ in $(seq 1 90); do
  # during initdb the entrypoint runs a socket-only server, so TCP readiness plus
  # the entrypoint's last log line means the real server is up
  if docker exec "$NAME" pg_isready -q -h 127.0.0.1 -U postgres 2>/dev/null \
     && docker logs "$NAME" 2>&1 | grep -qE 'init process complete|Skipping initialization'; then
    echo ok; exit 0
  fi
  printf .; sleep 2
done
echo "timed out"; docker logs --tail 50 "$NAME"; exit 1
