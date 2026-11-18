#!/usr/bin/env bash
# Robo Architect Lite on Supabase Postgres + Apache AGE, through the Bolt gateway.
#
#   ./run-robo.sh up        database, gateway, Robo's backend and frontend
#   ./run-robo.sh status
#   ./run-robo.sh down      stops what `up` started (data stays in the volume)
#   ./run-robo.sh purge     down, then removes the standalone database and its volume
#
# Robo Architect Lite itself is not changed: only its connection settings are
# given (NEO4J_URI / NEO4J_USER / NEO4J_PASSWORD / NEO4J_DATABASE, in the
# environment of the processes started here; its .env is left as it is).
#
# Where the database comes from
#   DB_CONTAINER=<name>   a running container of uengine/supabase-postgres-age
#                         (e.g. agesb-db, or a Supabase compose's db started with
#                         compose/supabase-age.override.yml); the AGE install and
#                         the graph database are made in it if missing (idempotent)
#   (unset)               compose/robo-age.yml, project robo-age: a database of
#                         its own on 127.0.0.1:${AGE_DB_PORT:-55449}
# Where the gateway comes from (GATEWAY)
#   native (default)      cargo build of ../bolt, run here on 127.0.0.1:$BOLT_PORT
#   docker                image uengine/ontological-bolt:age (gateway/build.sh)
#   none                  already running on $BOLT_PORT (e.g. the compose override's graph-bolt)
# What of Robo to start (ROBO_PARTS): api,ui (default) · api · none
#
# Other settings: ROBO_DIR (/Users/uengine/robo-architect-lite), ROBO_GRAPH (the
# project graph, robo), BOLT_PORT (7697), API_PORT (8000), UI_PORT (5173),
# SECRETS_DIR (~/.age-supabase/secrets: admin.password, robo.password, mode 0600),
# STATE_DIR (~/.age-supabase/robo: logs and pids), COMPOSE_PROJECT (robo-age: the
# standalone compose project, its containers <project>-db-1 / -graph-bolt-1).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"
CMD="${1:-up}"

ROBO_DIR="${ROBO_DIR:-/Users/uengine/robo-architect-lite}"
ROBO_GRAPH="${ROBO_GRAPH:-robo}"
ROBO_PARTS="${ROBO_PARTS:-api,ui}"
GATEWAY="${GATEWAY:-native}"
BOLT_PORT="${BOLT_PORT:-7697}"
API_PORT="${API_PORT:-8000}"
UI_PORT="${UI_PORT:-5173}"
AGE_DB_PORT="${AGE_DB_PORT:-55449}"
DB_CONTAINER="${DB_CONTAINER:-}"
SECRETS_DIR="${SECRETS_DIR:-$HOME/.age-supabase/secrets}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:-$SECRETS_DIR/admin.password}"
ROBO_PW_FILE="${ROBO_PW_FILE:-$SECRETS_DIR/robo.password}"
STATE_DIR="${STATE_DIR:-$HOME/.age-supabase/robo}"
AGE_DB_IMAGE="${AGE_DB_IMAGE:-uengine/supabase-postgres-age:15.8.1.060-age1.8.0}"
AGE_BOLT_IMAGE="${AGE_BOLT_IMAGE:-uengine/ontological-bolt:age}"
COMPOSE_PROJECT="${COMPOSE_PROJECT:-robo-age}"
COMPOSE=(docker compose -f "$here/compose/robo-age.yml" -p "$COMPOSE_PROJECT")

say() { printf '[run-robo] %s\n' "$*"; }
die() { printf '[run-robo] error: %s\n' "$*" >&2; exit 1; }
port_free() { ! lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
pid_alive() { [ -s "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }
wait_port() { for _ in $(seq 1 "${2:-120}"); do nc -z 127.0.0.1 "$1" 2>/dev/null && return 0; sleep 0.5; done; return 1; }

mkdir -p "$STATE_DIR"
umask 077
mkdir -p "$SECRETS_DIR"
chmod 700 "$SECRETS_DIR"

db_name() {   # the database container
  if [ -n "$DB_CONTAINER" ]; then echo "$DB_CONTAINER"; else echo "$COMPOSE_PROJECT-db-1"; fi
}

db_host_port() {   # host:port the database is published on
  local p
  p="$(docker port "$(db_name)" 5432/tcp 2>/dev/null | head -1)"
  [ -n "$p" ] || die "$(db_name) does not publish 5432"
  echo "${p/0.0.0.0/127.0.0.1}"
}

compose() {   # docker compose on the standalone project, its variables in a 0600 file for the call only
  local envf rc=0
  envf="$(mktemp "$SECRETS_DIR/.env.XXXXXX")"
  printf 'POSTGRES_PASSWORD=%s\nAGE_DB_PORT=%s\nAGE_BOLT_PORT=%s\nROBO_GRAPH=%s\nAGE_DB_IMAGE=%s\nAGE_BOLT_IMAGE=%s\n' \
    "$(cat "$ADMIN_PW_FILE" 2>/dev/null || echo unset)" "$AGE_DB_PORT" "$BOLT_PORT" "$ROBO_GRAPH" "$AGE_DB_IMAGE" "$AGE_BOLT_IMAGE" > "$envf"
  "${COMPOSE[@]}" --env-file "$envf" "$@" || rc=$?
  rm -f "$envf"
  return $rc
}

up_db() {
  if [ -n "$DB_CONTAINER" ]; then
    [ "$(docker inspect -f '{{.State.Running}}' "$DB_CONTAINER" 2>/dev/null)" = true ] || die "container $DB_CONTAINER is not running"
    [ -s "$ADMIN_PW_FILE" ] || die "ADMIN_PW_FILE ($ADMIN_PW_FILE) must hold its supabase_admin password"
  else
    [ -s "$ADMIN_PW_FILE" ] || python3 -c 'import secrets; print(secrets.token_urlsafe(24))' > "$ADMIN_PW_FILE"
    chmod 600 "$ADMIN_PW_FILE"
    say "database: compose/robo-age.yml (project $COMPOSE_PROJECT) on 127.0.0.1:$AGE_DB_PORT"
    compose up -d --wait db
  fi
  say "AGE in every database, the graph database (scripts/install-age.sh)"
  NAME="$(db_name)" ADMIN_PW_FILE="$ADMIN_PW_FILE" "$here/scripts/install-age.sh" | sed 's/^/    /'
  say "login role robo in graph (scripts/create-robo-role.sh; password in $ROBO_PW_FILE)"
  NAME="$(db_name)" ADMIN_PW_FILE="$ADMIN_PW_FILE" ROBO_PW_FILE="$ROBO_PW_FILE" DB=graph \
    "$here/scripts/create-robo-role.sh" | sed 's/^/    /'
}

up_gateway() {
  case "$GATEWAY" in
    none)
      wait_port "$BOLT_PORT" 10 || die "GATEWAY=none but nothing listens on $BOLT_PORT"
      ;;
    docker)
      docker image inspect "$AGE_BOLT_IMAGE" >/dev/null 2>&1 || die "no image $AGE_BOLT_IMAGE: run gateway/build.sh"
      if [ -z "$DB_CONTAINER" ]; then
        compose up -d --no-recreate graph-bolt
      else
        local net
        net="$(docker inspect -f '{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}' "$DB_CONTAINER" | awk '{print $1}')"
        docker rm -f "$COMPOSE_PROJECT-bolt" >/dev/null 2>&1 || true
        docker run -d --init --name "$COMPOSE_PROJECT-bolt" --memory 256m --restart unless-stopped --network "$net" \
          -p "127.0.0.1:$BOLT_PORT:7687" -e OG_BOLT_PGHOST="$DB_CONTAINER" -e OG_BOLT_GRAPH="$ROBO_GRAPH" \
          "$AGE_BOLT_IMAGE" >/dev/null
      fi
      ;;
    native)
      port_free "$BOLT_PORT" || die "port $BOLT_PORT is taken"
      export PATH="$HOME/.cargo/bin:$PATH"
      local target="${CARGO_TARGET_DIR:-$here/gateway/target/native}"
      say "gateway: cargo build ($repo/bolt)"
      (cd "$repo/bolt" && CARGO_TARGET_DIR="$target" cargo build --release --locked -q)
      local hp; hp="$(db_host_port)"
      OG_BOLT_BACKEND=age OG_BOLT_LISTEN="127.0.0.1:$BOLT_PORT" OG_BOLT_PGHOST="${hp%:*}" OG_BOLT_PGPORT="${hp##*:}" \
        OG_BOLT_PGDATABASE=graph OG_BOLT_GRAPH="$ROBO_GRAPH" \
        nohup "$target/release/ontological-bolt" > "$STATE_DIR/gateway.log" 2>&1 &
      echo $! > "$STATE_DIR/gateway.pid"
      ;;
    *) die "GATEWAY is native, docker or none" ;;
  esac
  wait_port "$BOLT_PORT" 60 || die "the gateway did not open $BOLT_PORT (see $STATE_DIR/gateway.log)"
  say "gateway: bolt://127.0.0.1:$BOLT_PORT"
}

smoke() {
  local py="$ROBO_DIR/.venv/bin/python"
  [ -x "$py" ] || { say "skipping the Bolt check (no $py)"; return 0; }
  NEO4J_PASSWORD="$(cat "$ROBO_PW_FILE")" "$py" -I - "$BOLT_PORT" "$ROBO_GRAPH" <<'PY'
import os, sys
from neo4j import GraphDatabase
port, graph = sys.argv[1], sys.argv[2]
with GraphDatabase.driver(f"bolt://127.0.0.1:{port}", auth=("robo", os.environ["NEO4J_PASSWORD"])) as d:
    info = d.get_server_info()
    with d.session(database=graph) as s:
        n = s.run("MATCH (n) RETURN count(n) AS n").single()["n"]
    print(f"    {info.agent}, graph {graph}: {n} nodes")
PY
}

robo_env() {
  printf 'NEO4J_URI=bolt://127.0.0.1:%s\nNEO4J_USER=robo\nNEO4J_DATABASE=%s\n' "$BOLT_PORT" "$ROBO_GRAPH"
}

up_robo() {
  [ "$ROBO_PARTS" = none ] && return 0
  [ -d "$ROBO_DIR/api" ] || die "ROBO_DIR ($ROBO_DIR) is not a Robo Architect Lite checkout"
  if [[ "$ROBO_PARTS" == *api* ]]; then
    port_free "$API_PORT" || die "port $API_PORT is taken (API_PORT=...)"
    say "Robo backend: $ROBO_DIR (uvicorn api.main:app) on 127.0.0.1:$API_PORT"
    (
      cd "$ROBO_DIR"
      env NEO4J_URI="bolt://127.0.0.1:$BOLT_PORT" NEO4J_USER=robo NEO4J_PASSWORD="$(cat "$ROBO_PW_FILE")" \
        NEO4J_DATABASE="$ROBO_GRAPH" \
        nohup .venv/bin/python -m uvicorn api.main:app --host 127.0.0.1 --port "$API_PORT" --timeout-graceful-shutdown 3 \
        > "$STATE_DIR/api.log" 2>&1 &
      echo $! > "$STATE_DIR/api.pid"
    )
    for _ in $(seq 1 120); do curl -sf "http://127.0.0.1:$API_PORT/api/health" >/dev/null 2>&1 && break; sleep 0.5; done
    curl -sf "http://127.0.0.1:$API_PORT/api/health" >/dev/null || die "the backend did not answer /api/health (see $STATE_DIR/api.log)"
  fi
  if [[ "$ROBO_PARTS" == *ui* ]]; then
    [ -d "$ROBO_DIR/frontend/node_modules" ] || die "$ROBO_DIR/frontend/node_modules missing: (cd frontend && npm install)"
    port_free "$UI_PORT" || die "port $UI_PORT is taken (UI_PORT=...)"
    say "Robo frontend: vite on http://127.0.0.1:$UI_PORT (API proxied to $API_PORT)"
    (
      cd "$ROBO_DIR/frontend"
      ARCHITECT_API_URL="http://127.0.0.1:$API_PORT" nohup npm run dev -- --host 127.0.0.1 --port "$UI_PORT" --strictPort \
        > "$STATE_DIR/ui.log" 2>&1 &
      echo $! > "$STATE_DIR/ui.pid"
    )
    wait_port "$UI_PORT" 120 || die "the frontend did not open $UI_PORT (see $STATE_DIR/ui.log)"
  fi
}

stop_pid() {
  local f="$STATE_DIR/$1.pid"
  if pid_alive "$f"; then
    local p; p="$(cat "$f")"
    pkill -TERM -P "$p" 2>/dev/null || true
    kill "$p" 2>/dev/null || true
    say "stopped $1 ($p)"
  fi
  rm -f "$f"
}

case "$CMD" in
  up)
    up_db
    up_gateway
    smoke
    up_robo
    say "ready"
    say "  graph   : bolt://127.0.0.1:$BOLT_PORT  user robo  password in $ROBO_PW_FILE  database $ROBO_GRAPH"
    [[ "$ROBO_PARTS" == *api* ]] && say "  backend : http://127.0.0.1:$API_PORT/api/health"
    [[ "$ROBO_PARTS" == *ui* ]] && say "  open    : http://127.0.0.1:$UI_PORT"
    say "  logs    : $STATE_DIR   stop: $0 down"
    ;;
  down)
    stop_pid ui
    stop_pid api
    stop_pid gateway
    if docker inspect "$COMPOSE_PROJECT-bolt" >/dev/null 2>&1; then
      docker rm -f "$COMPOSE_PROJECT-bolt" >/dev/null && say "removed $COMPOSE_PROJECT-bolt"
    fi
    if [ -z "$DB_CONTAINER" ] && docker inspect "$COMPOSE_PROJECT-db-1" >/dev/null 2>&1; then
      compose stop
      say "stopped compose project $COMPOSE_PROJECT (volume kept; $0 purge removes it with the data)"
    fi
    ;;
  status)
    for p in gateway api ui; do
      if pid_alive "$STATE_DIR/$p.pid"; then say "$p: running ($(cat "$STATE_DIR/$p.pid"))"; else say "$p: -"; fi
    done
    docker ps --filter name="$(db_name)" --filter name="$COMPOSE_PROJECT" --format '[run-robo] {{.Names}}: {{.Status}} {{.Ports}}'
    ;;
  env)
    robo_env
    ;;
  purge)
    [ -z "$DB_CONTAINER" ] || die "purge removes only the standalone project's containers and volume, not $DB_CONTAINER"
    "$0" down
    compose down -v
    say "removed compose project $COMPOSE_PROJECT and its volume"
    ;;
  *) die "usage: $0 up|down|status|env|purge" ;;
esac
