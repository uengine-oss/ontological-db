#!/usr/bin/env bash
# Bring up Ontological: PostgreSQL (in Docker) + the Studio (on the host).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTAINER="${OG_CONTAINER:-ontological-dev}"
PORT="${OG_PORT:-7474}"
PGPORT="${OG_PGPORT:-28816}"
BOLTPORT="${OG_BOLTPORT:-28687}"   # host port for the Bolt gateway (spec 011)
DB="${OG_DB:-og}"

say() { printf '\033[36m→\033[0m %s\n' "$1"; }

# --- container -------------------------------------------------------------
if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
    if docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER"; then
        say "starting container $CONTAINER"
        docker start "$CONTAINER" >/dev/null
    else
        say "creating container $CONTAINER"
        docker volume create ontological-target >/dev/null
        docker volume create ontological-cargo >/dev/null
        docker run -d --name "$CONTAINER" \
            -v "$ROOT":/work \
            -v ontological-target:/work/engine/target \
            -v ontological-cargo:/home/dev/.cargo/registry \
            -p "$PGPORT":"$PGPORT" -p "$BOLTPORT":7687 \
            -w /work ontological-dev:latest sleep infinity >/dev/null
        docker exec "$CONTAINER" bash -lc 'sudo chown -R dev /work/engine/target /home/dev/.cargo/registry'
    fi
fi

# --- postgres --------------------------------------------------------------
if ! docker exec "$CONTAINER" bash -lc "pg_isready -h localhost -p $PGPORT -q"; then
    say "starting PostgreSQL on $PGPORT"
    # cargo pgrx needs the crate's directory — /work has no Cargo.toml.
    docker exec "$CONTAINER" bash -lc "cd /work/engine && cargo pgrx start pg16" >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do
        docker exec "$CONTAINER" bash -lc "pg_isready -h localhost -p $PGPORT -q" && break
        sleep 1
    done
fi

# --- extension + demo data -------------------------------------------------
if ! docker exec "$CONTAINER" bash -lc \
     "psql -h localhost -p $PGPORT -lqt | cut -d'|' -f1 | grep -qw $DB"; then
    say "creating database $DB with the demo graph"
    docker exec "$CONTAINER" bash -lc "
        createdb -h localhost -p $PGPORT $DB
        psql -h localhost -p $PGPORT -d $DB -q -c 'CREATE EXTENSION ontological CASCADE'
        psql -h localhost -p $PGPORT -d $DB -q -f /work/examples/demo.sql" >/dev/null 2>&1
fi

# --- bolt gateway (spec 011) ------------------------------------------------
# Optional by design: nothing on the PostgreSQL path depends on it running.
if [ "${OG_BOLT:-1}" = "1" ]; then
    if ! docker exec "$CONTAINER" bash -lc "pgrep -f ontological-bolt >/dev/null"; then
        # The gateway defaults to loopback, because Bolt authenticates in the
        # clear. Here it binds every interface on purpose: it is inside the
        # container, and what decides exposure is the -p mapping above — which
        # publishes the container's 7687, not $BOLTPORT.
        say "starting Bolt gateway on localhost:$BOLTPORT"
        docker exec "$CONTAINER" bash -lc \
            "cd /work/bolt && cargo build --release -q" >/dev/null 2>&1 || true
        docker exec -d "$CONTAINER" bash -lc \
            "OG_BOLT_PGPORT=$PGPORT OG_BOLT_PGDATABASE=$DB OG_BOLT_ADVERTISED=localhost:$BOLTPORT \
             OG_BOLT_LISTEN=0.0.0.0:7687 \
             /work/bolt/target/release/ontological-bolt > /tmp/ontological-bolt.log 2>&1"
    fi
fi

# --- studio ----------------------------------------------------------------
if [ ! -d "$ROOT/portal/node_modules" ]; then
    say "installing portal dependencies"
    (cd "$ROOT/portal" && npm install --no-audit --no-fund >/dev/null)
fi

pkill -f "portal/server/index.js" 2>/dev/null || true
sleep 1
say "starting Ontological Studio on http://localhost:$PORT"
PGHOST=127.0.0.1 PGPORT="$PGPORT" PGDATABASE="$DB" PGUSER=dev PORT="$PORT" \
    nohup node "$ROOT/portal/server/index.js" > /tmp/ontological-studio.log 2>&1 &

for _ in $(seq 1 20); do
    if curl -sf -m 1 "http://localhost:$PORT/api/health" >/dev/null 2>&1; then
        printf '\033[32m✓\033[0m Ontological Studio  http://localhost:%s\n' "$PORT"
        printf '  postgres  localhost:%s/%s\n  log       /tmp/ontological-studio.log\n' "$PGPORT" "$DB"
        exit 0
    fi
    sleep 0.5
done

printf '\033[31m✗\033[0m studio did not come up — see /tmp/ontological-studio.log\n'
tail -20 /tmp/ontological-studio.log
exit 1
