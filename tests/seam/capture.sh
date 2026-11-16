#!/usr/bin/env bash
# Capture the generated-SQL mold of this tree into a folder — tests/seam/README.md.
#
#   tests/seam/capture.sh --pg-config <PG16>/bin/pg_config --port N --out DIR [options]
#
# Builds the engine package (unless --engine-pkg), copies that PostgreSQL into the work directory,
# installs the engine and the statement tracer (tests/seam/tracer) there, initdb's a throwaway
# cluster on --port with the tracer preloaded, runs tests/seam/capture.py against it and removes
# the cluster. The mold is made with PostgreSQL 16 (the golden copy in tests/seam/golden is).
#
# Options — each also read from the environment variable beside it:
#   --pg-config PATH    OG_PG_CONFIG      PostgreSQL 16's pg_config (default: pg_config on PATH)
#   --port N            OG_MOLD_PGPORT    the throwaway cluster's port — required: your card's PG port
#   --out DIR                             the mold folder to write (replaced)
#   --work DIR          OG_MOLD_WORK      cluster, PostgreSQL copy, raw traces, logs
#                                         default ${TMPDIR:-/tmp}/og-mold-<port>
#   --engine-pkg DIR    OG_ENGINE_PKG     a `cargo pgrx package` output to install instead of building
#                                         (scripts/test-all.sh leaves one in <its work>/pkg)
#   --python PATH       OG_PYTHON         an interpreter with psycopg2 and the neo4j driver (default python3)
#   --only LIST                           comma list of sources: corpus,sql,typeql,tutorials
#   --repeat 2                            capture twice into DIR and DIR.2 and say whether they are the same
#   --keep              OG_MOLD_KEEP=1    leave the cluster running
# Also CARGO_TARGET_DIR for the engine build. cargo-pgrx 0.19.2 must be on PATH to build.
#
# Exit status: 0 captured (and, with --repeat, the captures are identical), 1 a capture failed or
# the repeat differs, 2 bad usage or a missing prerequisite.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PG_CONFIG="${OG_PG_CONFIG:-}"
PORT="${OG_MOLD_PGPORT:-}"
OUT=""
WORK="${OG_MOLD_WORK:-}"
ENGINE_PKG="${OG_ENGINE_PKG:-}"
PY="${OG_PYTHON:-python3}"
ONLY=""
REPEAT=1
KEEP="${OG_MOLD_KEEP:-0}"

usage() { sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
while [ $# -gt 0 ]; do
    case "$1" in
        --pg-config) PG_CONFIG="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --engine-pkg) ENGINE_PKG="$2"; shift 2 ;;
        --python) PY="$2"; shift 2 ;;
        --only) ONLY="$2"; shift 2 ;;
        --repeat) REPEAT="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        -h|--help) usage ;;
        *) echo "unknown option: $1" >&2; usage ;;
    esac
done
[ -n "$OUT" ] || { echo "--out DIR is required" >&2; exit 2; }
[ -n "$PORT" ] || { echo "--port N is required (the card's own PostgreSQL port)" >&2; exit 2; }
case "$REPEAT" in 1|2) ;; *) echo "--repeat is 1 or 2" >&2; exit 2 ;; esac
WORK="${WORK:-${TMPDIR:-/tmp}/og-mold-$PORT}"
WORK="${WORK%/}"
mkdir -p "$WORK/logs" || exit 2
WORK="$(cd "$WORK" && pwd -P)"
case "$OUT" in /*) ;; *) OUT="$PWD/$OUT" ;; esac
LOGS="$WORK/logs"
say() { printf '\033[36m== %s\033[0m\n' "$*"; }

PG_CONFIG="${PG_CONFIG:-$(command -v pg_config || true)}"
[ -x "$PG_CONFIG" ] || { echo "no pg_config (give --pg-config)" >&2; exit 2; }
PG_VERSION="$("$PG_CONFIG" --version | awk '{print $2}')"
PG_MAJOR="${PG_VERSION%%.*}"
case "$PG_MAJOR" in 16|17) ;; *) echo "PostgreSQL $PG_VERSION: the tracer is written for 16 and 17" >&2; exit 2 ;; esac
"$PY" -c 'import psycopg2, neo4j' 2>/dev/null || { echo "$PY needs psycopg2 and the neo4j driver" >&2; exit 2; }
port_busy() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
if port_busy "$PORT"; then echo "port $PORT is already taken" >&2; exit 2; fi

# ------------------------------------------------------------------ engine
if [ -z "$ENGINE_PKG" ]; then
    say "engine: cargo pgrx package for pg$PG_MAJOR"
    command -v cargo-pgrx >/dev/null || { echo "cargo-pgrx is not on PATH" >&2; exit 2; }
    export PGRX_HOME="$WORK/pgrx-home"
    mkdir -p "$PGRX_HOME"
    printf '[configs]\npg%s = "%s"\n' "$PG_MAJOR" "$PG_CONFIG" > "$PGRX_HOME/config.toml"
    rm -rf "$WORK/pkg"
    if ! (cd "$ROOT/engine" && cargo pgrx package --no-default-features --features "pg$PG_MAJOR" \
            --pg-config "$PG_CONFIG" --out-dir "$WORK/pkg") > "$LOGS/build-engine.log" 2>&1; then
        tail -30 "$LOGS/build-engine.log"; echo "engine build failed" >&2; exit 1
    fi
    ENGINE_PKG="$WORK/pkg"
fi
lib="$(find "$ENGINE_PKG" -type f \( -name 'ontological.so' -o -name 'ontological.dylib' \) | head -1)"
ctl="$(find "$ENGINE_PKG" -type f -path '*/extension/ontological*' | head -20)"
[ -n "$lib" ] && [ -n "$ctl" ] || { echo "no engine in $ENGINE_PKG" >&2; exit 2; }

# ------------------------------------------------------------------ PostgreSQL copy + tracer
src="$(cd "$("$PG_CONFIG" --bindir)/.." && pwd -P)"
if [ "$src" != "$WORK/pg" ]; then
    say "copying $src to $WORK/pg"
    rm -rf "$WORK/pg"
    cp -Rc "$src" "$WORK/pg" 2>/dev/null || cp -R "$src" "$WORK/pg" || exit 2
fi
PGC="$WORK/pg/bin/pg_config"
export PATH="$WORK/pg/bin:$PATH"
cp "$lib" "$("$PGC" --pkglibdir)/" || exit 2
# shellcheck disable=SC2086
cp $ctl "$("$PGC" --sharedir)/extension/" || exit 2
rm -rf "$WORK/tracer"; mkdir -p "$WORK/tracer"
cp "$ROOT/tests/seam/tracer/og_mold_trace.c" "$ROOT/tests/seam/tracer/Makefile" "$WORK/tracer/"
if ! make -C "$WORK/tracer" PG_CONFIG="$PGC" > "$LOGS/build-tracer.log" 2>&1; then
    cat "$LOGS/build-tracer.log"; echo "tracer build failed" >&2; exit 1
fi
cp "$WORK/tracer"/og_mold_trace.* "$("$PGC" --pkglibdir)/" 2>/dev/null
rm -f "$("$PGC" --pkglibdir)/og_mold_trace.o" "$("$PGC" --pkglibdir)/og_mold_trace.c"

# ------------------------------------------------------------------ cluster
DATA="$WORK/data"
cleanup() {
    if [ -f "$DATA/postmaster.pid" ]; then
        if [ "$KEEP" = 1 ]; then echo "cluster left running: pg_ctl -D $DATA -m fast stop; rm -rf $DATA"
        else pg_ctl -D "$DATA" -m fast -w stop >/dev/null 2>&1; rm -rf "$DATA"; fi
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

capture_once() {  # out-dir
    rm -rf "$DATA"
    initdb -D "$DATA" -U og -A trust -E UTF8 --locale=C --no-instructions > "$LOGS/initdb.log" 2>&1 \
        || { cat "$LOGS/initdb.log"; return 2; }
    cat >> "$DATA/postgresql.conf" <<EOF

# tests/seam/capture.sh
port = $PORT
listen_addresses = 'localhost'
unix_socket_directories = ''
timezone = 'UTC'
log_timezone = 'UTC'
fsync = off
session_preload_libraries = 'og_mold_trace'
EOF
    cat > "$DATA/pg_hba.conf" <<'EOF'
local   all   all                        trust
host    all   og_fabric_pw   127.0.0.1/32   scram-sha-256
host    all   og_fabric_pw   ::1/128        scram-sha-256
host    all   all            127.0.0.1/32   trust
host    all   all            ::1/128        trust
EOF
    # started without PGOPTIONS, so the connections postgres_fdw opens are not traced
    env -u PGOPTIONS pg_ctl -D "$DATA" -l "$LOGS/postgres.log" -w start >/dev/null || { tail -20 "$LOGS/postgres.log"; return 2; }
    psql -X -q -h 127.0.0.1 -p "$PORT" -U og -d postgres -c "CREATE ROLE dev LOGIN SUPERUSER" || return 2
    local t0; t0=$(date +%s)
    local only=(); [ -n "$ONLY" ] && only=(--only "$ONLY")
    "$PY" "$ROOT/tests/seam/capture.py" --port "$PORT" --psql "$WORK/pg/bin/psql" --out "$1" \
        --work "$WORK/run" --data-dir "$DATA" ${only[@]+"${only[@]}"}
    local rc=$?
    echo "capture: $(( $(date +%s) - t0 ))s → $1"
    return $rc
}
stop_cluster() { pg_ctl -D "$DATA" -m fast -w stop >/dev/null 2>&1; rm -rf "$DATA"; }

say "capture (PostgreSQL $PG_VERSION, port $PORT)"
capture_once "$OUT" || exit 1
if [ "$REPEAT" = 2 ]; then
    say "capture again, to check it is deterministic"
    stop_cluster
    capture_once "$OUT.2" || exit 1
    if diff -r -q "$OUT" "$OUT.2"; then echo "identical: $OUT == $OUT.2"
    else echo "DIFFERENT: $OUT vs $OUT.2"; exit 1; fi
fi
