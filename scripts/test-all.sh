#!/usr/bin/env bash
# Every suite, against a throwaway PostgreSQL cluster — the run CI does, done the same way here.
#
#   scripts/test-all.sh --edition v1 --pg 16 --pg-config /path/to/bin/pg_config [options]
#
# Builds the engine package and the Bolt gateway (unless given ready-made), installs the engine
# into that PostgreSQL, initdb's a scratch cluster on --port, then runs in order: the Bolt unit
# tests, the SQL regression (tests/run.sh: tests/common/sql + tests/<edition>/sql, then the backup
# round trip), the Python suites that start their own gateway, TypeQL, the Neo4j Movie Graph,
# the Neo4j compatibility fixes, and the tutorials; prints a summary with passed/expected for
# each. The cluster is stopped and its data directory removed at the end unless --keep.
# Which tests belong to which edition: tests/README.md.
#
# Options — each also read from the environment variable beside it:
#   --edition v1|age     OG_EDITION        default v1. age has no engine yet: every step is reported
#                                          as skipped and the run succeeds.
#   --pg 16|17           OG_PG_MAJOR       the PostgreSQL major this run is for; checked against pg_config.
#   --pg-config PATH     OG_PG_CONFIG      default: pg_config on PATH.
#   --copy-pg            OG_COPY_PG=1      copy that PostgreSQL tree into the work directory and install
#                                          there (for a tree the run must not write to, e.g. the Studio's
#                                          runtime/pg).
#   --sudo               OG_INSTALL_SUDO=1 install the engine with sudo (a system PostgreSQL, as in CI).
#   --port N             OG_TEST_PGPORT    the cluster's port, default 28816.
#   --bolt-base N        OG_TEST_BOLT_BASE first of the twelve gateway ports the suites use, default 27688
#                                          (the suites' own defaults: bolt-context 27688 … untyped 27699).
#   --work DIR           OG_TEST_WORK      data directory, logs, dump, build output.
#                                          default ${TMPDIR:-/tmp}/og-test-<edition>-pg<major>-<port>
#   --engine-pkg DIR     OG_ENGINE_PKG     install this `cargo pgrx package` output instead of building.
#   --bolt-bin PATH      OG_BOLT_BIN       use this gateway binary instead of building one.
#   --python PATH        OG_PYTHON         an interpreter with neo4j (5 or later) and psycopg2; default python3.
#   --only LIST          OG_TEST_ONLY      comma list of steps: unit,sql,python,typeql,movies,compat,tutorials
#   --keep               OG_TEST_KEEP=1    leave the cluster running and say how to stop it.
# Also: OG_TEST_TZ (the cluster's TimeZone, default UTC), OG_MOVIES_CYPHER (a local copy of the Movie
# Graph's movies.cypher, used instead of downloading it), OG_TEST_MYSQL_* (passed to
# fabric-mysql-catalog; without OG_TEST_MYSQL_HOST that suite is skipped), CARGO_TARGET_DIR.
#
# Exit status: 0 when nothing failed (skipped steps do not fail a run), 1 otherwise, 2 on bad usage
# or a missing prerequisite.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

EDITION="${OG_EDITION:-v1}"
PG_MAJOR="${OG_PG_MAJOR:-}"
PG_CONFIG="${OG_PG_CONFIG:-}"
COPY_PG="${OG_COPY_PG:-0}"
SUDO_INSTALL="${OG_INSTALL_SUDO:-0}"
PORT="${OG_TEST_PGPORT:-28816}"
BOLT_BASE="${OG_TEST_BOLT_BASE:-27688}"
WORK="${OG_TEST_WORK:-}"
ENGINE_PKG="${OG_ENGINE_PKG:-}"
BOLT_BIN="${OG_BOLT_BIN:-}"
PY="${OG_PYTHON:-python3}"
ONLY="${OG_TEST_ONLY:-}"
KEEP="${OG_TEST_KEEP:-0}"
TZ_NAME="${OG_TEST_TZ:-UTC}"

usage() { sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
while [ $# -gt 0 ]; do
    case "$1" in
        --edition) EDITION="$2"; shift 2 ;;
        --pg) PG_MAJOR="$2"; shift 2 ;;
        --pg-config) PG_CONFIG="$2"; shift 2 ;;
        --copy-pg) COPY_PG=1; shift ;;
        --sudo) SUDO_INSTALL=1; shift ;;
        --port) PORT="$2"; shift 2 ;;
        --bolt-base) BOLT_BASE="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --engine-pkg) ENGINE_PKG="$2"; shift 2 ;;
        --bolt-bin) BOLT_BIN="$2"; shift 2 ;;
        --python) PY="$2"; shift 2 ;;
        --only) ONLY="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        -h|--help) usage ;;
        *) echo "unknown option: $1" >&2; usage ;;
    esac
done
case "$EDITION" in v1|age) ;; *) echo "--edition must be v1 or age, not '$EDITION'" >&2; exit 2 ;; esac
[ -n "$PG_MAJOR" ] || { echo "--pg 16|17 is required" >&2; exit 2; }
WORK="${WORK:-${TMPDIR:-/tmp}/og-test-$EDITION-pg$PG_MAJOR-$PORT}"
WORK="${WORK%/}"
LOGS="$WORK/logs"
SUMMARY="$WORK/summary.txt"

# ------------------------------------------------------------------ results
# One line per step: step|status|passed|expected|note ('|', not a tab: read would merge empty fields).
# status: ok | FAIL | MISMATCH (exit 0 but not the expected count) | known (below) | skip
mkdir -p "$LOGS"
: > "$SUMMARY"

# Known failures, each with its reason (tests/README.md, 「알려진 실패」): "<passed count>|<reason>".
# A step that fails with exactly that count is reported as "known" and does not fail the run; any
# other count — better or worse — stays a FAIL, so the gate still sees a change. When the step
# passes again the summary says so, and the entry should go.
known_failure() {
    case "$EDITION/$1" in
        v1/neo4j-movies) echo "30|Studio patch 0012 (92b4467): MERGE of a relationship whose list property is declared text[] compiles (\$1 ->> '__og_merge_0')::text[], a malformed array literal (engine/src/cypher/mod.rs:754); the dataset loads 1/39" ;;
    esac
}
record() {
    local st="$2" note="${5:-}" k
    k="$(known_failure "$1")"
    if [ -n "$k" ]; then
        if [ "$st" = FAIL ] && [ "$3" = "${k%%|*}" ]; then st=known; note="known failure — ${k#*|}"
        elif [ "$st" = ok ]; then note="$note (passes now: remove it from known_failure in scripts/test-all.sh)"
        fi
    fi
    printf '%s|%s|%s|%s|%s\n' "$1" "$st" "$3" "$4" "$(printf '%s' "$note" | tr '|\n' '/ ')" >> "$SUMMARY"
}
say() { printf '\n\033[36m== %s\033[0m\n' "$*"; }
want() { [ -z "$ONLY" ] || case ",$ONLY," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# The counts each suite reached when it was last known good (P0-a, v1 on PostgreSQL 16).
# A suite that exits 0 with fewer checks than this has changed, and that is worth a look.
expected() {
    case "$1" in
        bolt-unit) echo 56 ;;
        bolt-context) echo 9 ;;         bolt-timeout) echo 15 ;;
        bolt-cypher-gaps) echo 19 ;;    bolt-concurrency) echo 9 ;;
        bolt-fabric-query) echo 21 ;;   bolt-multibyte) echo 9 ;;
        bolt-params) echo 12 ;;         bolt-subqueries) echo 19 ;;
        fabric-mysql-catalog) echo 8 ;; bolt-untyped-virtual) echo 15 ;;
        neo4j-movies) echo 33 ;;        # 24 guide queries + 9 driver checks
        *) echo "" ;;
    esac
}
# Gateway port of each suite: the suite's own default when the base is 27688.
bolt_port() {
    case "$1" in
        bolt-context) echo $((BOLT_BASE + 0)) ;;      bolt-timeout) echo $((BOLT_BASE + 1)) ;;
        bolt-cypher-gaps) echo $((BOLT_BASE + 2)) ;;  bolt-concurrency) echo $((BOLT_BASE + 3)) ;;
        bolt-fabric-query) echo $((BOLT_BASE + 4)) ;; bolt-multibyte) echo $((BOLT_BASE + 5)) ;;
        bolt-params) echo $((BOLT_BASE + 6)) ;;       neo4j-movies) echo $((BOLT_BASE + 7)) ;;
        neo4j-compat-fixes) echo $((BOLT_BASE + 8)) ;; bolt-subqueries) echo $((BOLT_BASE + 9)) ;;
        fabric-mysql-catalog) echo $((BOLT_BASE + 10)) ;; bolt-untyped-virtual) echo $((BOLT_BASE + 11)) ;;
        *) echo "" ;;
    esac
}

print_summary() {
    say "summary — edition $EDITION, PostgreSQL ${PG_VERSION:-$PG_MAJOR}"
    local bad=0
    printf '%-26s %-9s %9s  %s\n' step result "passed" note
    while IFS='|' read -r step status passed exp note; do
        local count="$passed"; [ -n "$exp" ] && count="$passed/$exp"
        printf '%-26s %-9s %9s  %s\n' "$step" "$status" "$count" "$note"
        case "$status" in FAIL|MISMATCH) bad=1 ;; esac
    done < "$SUMMARY"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### Ontological tests — $EDITION × PostgreSQL ${PG_VERSION:-$PG_MAJOR}"
            echo
            echo "| step | result | passed | note |"
            echo "|---|---|---|---|"
            while IFS='|' read -r step status passed exp note; do
                local count="$passed"; [ -n "$exp" ] && count="$passed/$exp"
                echo "| $step | $status | $count | $note |"
            done < "$SUMMARY"
        } >> "$GITHUB_STEP_SUMMARY"
    fi
    echo
    echo "logs: $LOGS"
    return $bad
}

# ------------------------------------------------------------------ age: not yet
if [ "$EDITION" = age ]; then
    for s in unit sql python typeql movies compat tutorials; do
        record "$s" skip "" "" "the age edition (ontological_age) is not built yet"
    done
    print_summary
    exit 0
fi

# ------------------------------------------------------------------ PostgreSQL
PG_CONFIG="${PG_CONFIG:-$(command -v pg_config || true)}"
[ -x "$PG_CONFIG" ] || { echo "no pg_config (give --pg-config)" >&2; exit 2; }
PG_VERSION="$("$PG_CONFIG" --version | awk '{print $2}')"
case "$PG_VERSION" in
    "$PG_MAJOR".*) ;;
    *) echo "$PG_CONFIG is PostgreSQL $PG_VERSION, not $PG_MAJOR" >&2; exit 2 ;;
esac
if [ "$COPY_PG" = 1 ]; then
    src="$(cd "$("$PG_CONFIG" --bindir)/.." && pwd)"
    if [ ! -x "$WORK/pg/bin/postgres" ]; then
        say "copying $src to $WORK/pg"
        rm -rf "$WORK/pg"; cp -R "$src" "$WORK/pg" || exit 2
    fi
    PG_CONFIG="$WORK/pg/bin/pg_config"
fi
PG_BIN="$("$PG_CONFIG" --bindir)"
export PATH="$PG_BIN:$PATH"

port_busy() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
if port_busy "$PORT"; then echo "port $PORT is already taken" >&2; exit 2; fi

# ------------------------------------------------------------------ build
if [ -z "$ENGINE_PKG" ] && { want sql || want python || want typeql || want movies || want compat || want tutorials; }; then
    say "engine: cargo pgrx package for pg$PG_MAJOR"
    command -v cargo-pgrx >/dev/null || { echo "cargo-pgrx is not on PATH (cargo install cargo-pgrx --version 0.19.2 --locked)" >&2; exit 2; }
    export PGRX_HOME="$WORK/pgrx-home"
    mkdir -p "$PGRX_HOME"
    printf '[configs]\npg%s = "%s"\n' "$PG_MAJOR" "$PG_CONFIG" > "$PGRX_HOME/config.toml"
    rm -rf "$WORK/pkg"
    if (cd "$ROOT/engine" && cargo pgrx package --no-default-features --features "pg$PG_MAJOR" \
            --pg-config "$PG_CONFIG" --out-dir "$WORK/pkg") > "$LOGS/build-engine.log" 2>&1; then
        ENGINE_PKG="$WORK/pkg"
        record build-engine ok "" "" "pg$PG_MAJOR"
    else
        tail -30 "$LOGS/build-engine.log"
        record build-engine FAIL "" "" "see build-engine.log"
        print_summary; exit 1
    fi
fi
if [ -z "$BOLT_BIN" ] && { want unit || want python || want movies || want compat; }; then
    say "bolt: cargo build --release --locked"
    if cargo build --release --locked --manifest-path "$ROOT/bolt/Cargo.toml" > "$LOGS/build-bolt.log" 2>&1; then
        BOLT_BIN="${CARGO_TARGET_DIR:-$ROOT/bolt/target}/release/ontological-bolt"
        record build-bolt ok "" "" ""
    else
        tail -30 "$LOGS/build-bolt.log"
        record build-bolt FAIL "" "" "see build-bolt.log"
        print_summary; exit 1
    fi
fi

# ------------------------------------------------------------------ Bolt unit tests
if want unit; then
    say "bolt unit tests"
    if cargo test --release --locked --manifest-path "$ROOT/bolt/Cargo.toml" > "$LOGS/bolt-unit.log" 2>&1; then st=ok; else st=FAIL; fi
    n="$(grep -E '^test result:' "$LOGS/bolt-unit.log" | sed -E 's/.* ([0-9]+) passed.*/\1/' | awk '{s+=$1} END {print s+0}')"
    [ "$st" = ok ] && [ "$n" != "$(expected bolt-unit)" ] && st=MISMATCH
    record bolt-unit "$st" "$n" "$(expected bolt-unit)" "cargo test (bolt/)"
fi

if ! { want sql || want python || want typeql || want movies || want compat || want tutorials; }; then
    print_summary; exit $?
fi

# ------------------------------------------------------------------ install
say "installing the engine into $("$PG_CONFIG" --pkglibdir)"
SUDO=""; [ "$SUDO_INSTALL" = 1 ] && SUDO="sudo"
lib="$(find "$ENGINE_PKG" -type f \( -name 'ontological.so' -o -name 'ontological.dylib' \) | head -1)"
ctl="$(find "$ENGINE_PKG" -type f -path '*/extension/ontological*' | head -20)"
if [ -z "$lib" ] || [ -z "$ctl" ]; then echo "no engine in $ENGINE_PKG" >&2; exit 2; fi
$SUDO cp "$lib" "$("$PG_CONFIG" --pkglibdir)/" || exit 2
# shellcheck disable=SC2086
$SUDO cp $ctl "$("$PG_CONFIG" --sharedir)/extension/" || exit 2
for need in vector postgres_fdw; do
    [ -f "$("$PG_CONFIG" --sharedir)/extension/$need.control" ] || { echo "PostgreSQL has no $need extension" >&2; exit 2; }
done

# ------------------------------------------------------------------ cluster
DATA="$WORK/data"
GATEWAYS=""
cleanup() {
    for p in $GATEWAYS; do kill "$p" 2>/dev/null; done
    if [ -f "$DATA/postmaster.pid" ]; then
        if [ "$KEEP" = 1 ]; then
            echo "cluster left running: pg_ctl -D $DATA -m fast stop; rm -rf $DATA"
        else
            pg_ctl -D "$DATA" -m fast -w stop >/dev/null 2>&1
            rm -rf "$DATA"
        fi
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

say "cluster: PostgreSQL $PG_VERSION on 127.0.0.1:$PORT, $DATA"
rm -rf "$DATA"
initdb -D "$DATA" -U og -A trust -E UTF8 --locale=C --no-instructions > "$LOGS/initdb.log" 2>&1 \
    || { cat "$LOGS/initdb.log"; exit 2; }
cat >> "$DATA/postgresql.conf" <<EOF

# scripts/test-all.sh
port = $PORT
listen_addresses = 'localhost'
unix_socket_directories = ''
timezone = '$TZ_NAME'
log_timezone = '$TZ_NAME'
fsync = off
EOF
# The fabric suites (SQL 15 and 23, bolt-fabric-query) need a source account that must log in
# with a password; everyone else is trusted on loopback.
cat > "$DATA/pg_hba.conf" <<'EOF'
local   all   all                        trust
host    all   og_fabric_pw   127.0.0.1/32   scram-sha-256
host    all   og_fabric_pw   ::1/128        scram-sha-256
host    all   all            127.0.0.1/32   trust
host    all   all            ::1/128        trust
EOF
pg_ctl -D "$DATA" -l "$LOGS/postgres.log" -w start >/dev/null || { tail -20 "$LOGS/postgres.log"; exit 2; }

export PGHOST=127.0.0.1 PGPORT="$PORT" PGUSER=og
Q() { psql -X -q -v ON_ERROR_STOP=1 -tA -d "${2:-postgres}" -c "$1"; }
# The tutorials were written in the dev container, whose superuser is dev, and one of them
# leaves that user's own writes out by name (security: db_user <> 'dev'). They run as dev.
Q "CREATE ROLE dev LOGIN SUPERUSER" >/dev/null || exit 2

# ------------------------------------------------------------------ SQL regression + backup round trip
if want sql; then
    say "SQL regression (tests/run.sh, OG_EDITION=$EDITION)"
    OG_EDITION="$EDITION" OG_DUMP_FILE="$WORK/og_roundtrip.dump" bash "$ROOT/tests/run.sh" 2>&1 | tee "$LOGS/sql.log"
    rc=${PIPESTATUS[0]}
    nfiles="$(find "$ROOT/tests/common/sql" "$ROOT/tests/$EDITION/sql" -maxdepth 1 -name '*.sql' 2>/dev/null | wc -l | tr -d ' ')"
    nok="$(grep -cE '^[0-9A-Za-z_]+\.sql *ok$' "$LOGS/sql.log")"
    ncommon="$(find "$ROOT/tests/common/sql" -maxdepth 1 -name '*.sql' | wc -l | tr -d ' ')"
    st=ok; [ "$nok" = "$nfiles" ] || st=FAIL
    record sql-regression "$st" "$nok" "$nfiles" "common $ncommon + $EDITION $((nfiles - ncommon))"
    if [ "$EDITION" = v1 ]; then
        if grep -qE '^backup round trip +ok' "$LOGS/sql.log"; then st=ok; else st=FAIL; fi
        record backup-round-trip "$st" "$([ $st = ok ] && echo 1 || echo 0)" 1 \
            "$(grep -E '^backup round trip' "$LOGS/sql.log" | sed -E 's/^backup round trip +//')"
        if grep -qE '^integrity +ok' "$LOGS/sql.log"; then st=ok; else st=FAIL; fi
        record integrity "$st" "" "" "og_check_integrity() after the round trip"
    fi
    [ "$rc" = 0 ] || grep -q FAIL "$SUMMARY" || record sql-regression FAIL "" "" "tests/run.sh exited $rc"
fi

# ------------------------------------------------------------------ Python
py_ok=1
if want python || want typeql || want movies || want compat; then
    if ! "$PY" -c 'import sys, neo4j, psycopg2; sys.exit(int(neo4j.__version__.split(".")[0]) < 5)' 2>/dev/null; then
        echo "$PY needs the neo4j driver (5 or later) and psycopg2" >&2
        py_ok=0
    fi
fi
run_counted() {  # name expected logfile command… — counts the suite's "ok   " lines
    local name="$1" exp="$2" log="$3"; shift 3
    say "$name"
    "$@" > "$log" 2>&1; local rc=$?
    local n; n="$(grep -c '^ok   ' "$log")"
    local st=ok note=""
    if [ $rc != 0 ]; then st=FAIL; note="$(grep -E '^FAIL|Error|error' "$log" | head -1 | cut -c1-110)"
    elif grep -q 'skipped' "$log" && [ "$n" = 0 ]; then st=skip; note="$(grep skipped "$log" | head -1 | cut -c1-110)"
    elif [ -n "$exp" ] && [ "$n" != "$exp" ]; then st=MISMATCH
    fi
    tail -3 "$log"
    record "$name" "$st" "$n" "$exp" "$note"
}

if want python; then
    for dir in "$ROOT"/tests/common/*/ "$ROOT"/tests/"$EDITION"/*/; do
        [ -f "$dir/run.py" ] || continue
        name="$(basename "$dir")"
        case "$name" in typeql|neo4j-movies|neo4j-compat-fixes) continue ;; esac
        if [ $py_ok = 0 ]; then record "$name" FAIL "" "$(expected "$name")" "no usable Python"; continue; fi
        bp="$(bolt_port "$name")"
        [ -n "$bp" ] || { record "$name" FAIL "" "" "no Bolt port assigned in scripts/test-all.sh"; continue; }
        if port_busy "$bp"; then record "$name" FAIL "" "$(expected "$name")" "port $bp is taken"; continue; fi
        run_counted "$name" "$(expected "$name")" "$LOGS/$name.log" \
            env OG_BOLT_BIN="$BOLT_BIN" OG_BOLT_TEST_LISTEN="127.0.0.1:$bp" "$PY" "$dir/run.py"
    done
fi

# ------------------------------------------------------------------ TypeQL
if want typeql; then
    for dir in "$ROOT"/tests/common/typeql "$ROOT"/tests/"$EDITION"/typeql; do
        [ -f "$dir/run.py" ] || continue
        say "typeql ($(basename "$(dirname "$dir")"))"
        if [ $py_ok = 0 ]; then record typeql FAIL "" "" "no usable Python"; continue; fi
        OG_TYPEQL_DUMP="$WORK/og_typeql_dump.tql" "$PY" "$dir/run.py" > "$LOGS/typeql.log" 2>&1; rc=$?
        tail -4 "$LOGS/typeql.log"
        line="$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOGS/typeql.log" | grep -E '^[0-9]+/[0-9]+ checks passed' | tail -1)"
        p="$(printf '%s' "$line" | sed -E 's#^([0-9]+)/([0-9]+).*#\1#')"
        t="$(printf '%s' "$line" | sed -E 's#^([0-9]+)/([0-9]+).*#\2#')"
        st=ok; [ $rc = 0 ] && [ -n "$p" ] && [ "$p" = "$t" ] || st=FAIL
        record typeql "$st" "$p" "$t" "$(printf '%s' "$line" | sed -E 's#^[0-9]+/[0-9]+ checks passed##; s/^[ ,(]*//; s/[)]$//')"
    done
fi

start_gateway() {  # name port database graph — the gateway for a suite that does not start its own
    if port_busy "$2"; then echo "port $2 is taken" >&2; return 1; fi
    OG_BOLT_LISTEN="127.0.0.1:$2" OG_BOLT_PGHOST=127.0.0.1 OG_BOLT_PGPORT="$PORT" \
        OG_BOLT_PGDATABASE="$3" OG_BOLT_GRAPH="$4" "$BOLT_BIN" > "$LOGS/$1-gateway.log" 2>&1 &
    GW_PID=$!; GATEWAYS="$GATEWAYS $GW_PID"
    for _ in $(seq 1 60); do port_busy "$2" && return 0; sleep 0.5; done
    echo "the gateway did not start" >&2; return 1
}
stop_gateway() { kill "$GW_PID" 2>/dev/null; wait "$GW_PID" 2>/dev/null; }
fresh_db() { Q "DROP DATABASE IF EXISTS $1 WITH (FORCE)" && Q "CREATE DATABASE $1" && Q "CREATE EXTENSION ontological CASCADE" "$1"; }

# ------------------------------------------------------------------ Neo4j Movie Graph
if want movies && [ -f "$ROOT/tests/common/neo4j-movies/run.py" ]; then
    say "neo4j-movies (Ontological over PostgreSQL and Bolt; no Neo4j comparison)"
    MOVIES="$ROOT/tests/common/neo4j-movies"
    if [ ! -f "$MOVIES/movies.cypher" ] && [ -n "${OG_MOVIES_CYPHER:-}" ]; then cp "$OG_MOVIES_CYPHER" "$MOVIES/movies.cypher"; fi
    bp="$(bolt_port neo4j-movies)"
    if [ $py_ok = 0 ]; then record neo4j-movies FAIL "" 33 "no usable Python"
    elif fresh_db og_movies >/dev/null && start_gateway neo4j-movies "$bp" og_movies movies; then
        "$PY" "$MOVIES/run.py" --dsn "host=127.0.0.1 port=$PORT dbname=og_movies user=og" \
            --pg-host 127.0.0.1 --pg-port "$PORT" --bolt "bolt://127.0.0.1:$bp" --bolt-user og \
            --no-neo4j > "$LOGS/neo4j-movies.log" 2>&1; rc=$?
        stop_gateway
        plain="$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOGS/neo4j-movies.log")"
        n="$(printf '%s\n' "$plain" | grep -c '✓')"
        st=ok; [ $rc = 0 ] || st=FAIL
        [ $st = ok ] && [ "$n" != "$(expected neo4j-movies)" ] && st=MISMATCH
        printf '%s\n' "$plain" | tail -3
        record neo4j-movies "$st" "$n" "$(expected neo4j-movies)" \
            "$(printf '%s\n' "$plain" | grep -E 'loaded [0-9]+/[0-9]+ statements' | sed -E 's/^ +//')"
    else
        record neo4j-movies FAIL "" 33 "setup failed"
    fi
fi

# ------------------------------------------------------------------ Neo4j compatibility fixes
if want compat && [ -f "$ROOT/tests/common/neo4j-compat-fixes/run.py" ]; then
    say "neo4j-compat-fixes"
    bp="$(bolt_port neo4j-compat-fixes)"
    if [ $py_ok = 0 ]; then record neo4j-compat-fixes FAIL "" "" "no usable Python"
    elif fresh_db og_compat >/dev/null && Q "SELECT og_create_graph('fix_probe')" og_compat >/dev/null \
            && start_gateway neo4j-compat-fixes "$bp" og_compat fix_probe; then
        OG_BOLT_URI="bolt://127.0.0.1:$bp" OG_BOLT_USER=og \
            "$PY" "$ROOT/tests/common/neo4j-compat-fixes/run.py" > "$LOGS/neo4j-compat-fixes.log" 2>&1; rc=$?
        stop_gateway
        p="$(grep -c '\[PASS\]' "$LOGS/neo4j-compat-fixes.log")"
        f="$(grep -c '\[FAIL\]' "$LOGS/neo4j-compat-fixes.log")"
        st=ok; [ $rc = 0 ] && [ "$f" = 0 ] || st=FAIL
        tail -3 "$LOGS/neo4j-compat-fixes.log"
        record neo4j-compat-fixes "$st" "$p" "$((p + f))" ""
    else
        record neo4j-compat-fixes FAIL "" "" "setup failed"
    fi
fi

# ------------------------------------------------------------------ tutorials
# All six tutorials tests/tutorials.sh runs read v1's tables (tests/README.md), so they are v1's.
if want tutorials && [ "$EDITION" = v1 ]; then
    say "tutorials (tests/tutorials.sh)"
    (cd "$ROOT" && PGUSER=dev TUT_EXEC=env TUT_PGPORT="$PORT" bash tests/tutorials.sh) > "$LOGS/tutorials.log" 2>&1; rc=$?
    line="$(grep -E '^[0-9]+ of [0-9]+ tutorials passed' "$LOGS/tutorials.log" | tail -1)"
    p="$(printf '%s' "$line" | awk '{print $1}')"; t="$(printf '%s' "$line" | awk '{print $3}')"
    st=ok; [ $rc = 0 ] || st=FAIL
    grep -E ': (ok|FAILED)|blocks failed' "$LOGS/tutorials.log" | sed 's/^/  /'
    drift="$(grep -c 'lines differ' "$LOGS/tutorials.log")"
    record tutorials "$st" "$p" "$t" "$([ "$drift" != 0 ] && echo "$drift drift from docs/tutorials (fails only with --strict)")"
fi

print_summary
