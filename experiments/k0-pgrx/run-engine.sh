#!/usr/bin/env bash
# K0 spike, real engine code: engine/src (unmodified) compiled as the library og_engine_lib, shipped
# by two thin extension crates. Nothing under engine/ is written (cargo only writes CARGO_TARGET_DIR).
#
#   experiments/k0-pgrx/run-engine.sh --pg 16|17 --pg-dir DIR [--build-pg-config PATH] [--port N] [--work DIR] [--keep]
#
# Same options and environment as run.sh. --pg-dir must already have pgvector (the v1 control
# requires it). Steps:
#   1. `cargo pgrx schema` of engine/ itself                      -> the install SQL as it is today
#   2. `cargo pgrx package` of engine_v1 (extension ontological)   -> must equal 1 apart from comments
#   3. `cargo pgrx package` of engine_age (extension ontological_age, schema oga_catalog); its script
#      is cut down to the pgrx functions (engine/sql/*.sql are v1's own SQL)
#   4. both into one database: sql/engine.sql
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="$(cd "$HERE/../../engine" && pwd)"
MAJ=""; PGDIR=""; BUILD_PGC=""; PORT=28820; WORK=""; KEEP=0
while [ $# -gt 0 ]; do
    case "$1" in
        --pg) MAJ="$2"; shift 2 ;;
        --pg-dir) PGDIR="$2"; shift 2 ;;
        --build-pg-config) BUILD_PGC="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        *) sed -n '2,14p' "$0"; exit 2 ;;
    esac
done
[ -n "$MAJ" ] && [ -n "$PGDIR" ] || { sed -n '2,14p' "$0"; exit 2; }
BUILD_PGC="${BUILD_PGC:-$PGDIR/bin/pg_config}"
WORK="${WORK:-${TMPDIR:-/tmp}/k0-pgrx-engine-pg$MAJ}"
OUT="$HERE/results/pg$MAJ"
mkdir -p "$WORK" "$OUT"
export PATH="$PGDIR/bin:$PATH"
if [ -z "${PGRX_HOME:-}" ]; then
    export PGRX_HOME="$WORK/pgrx-home"; mkdir -p "$PGRX_HOME"
    printf '[configs]\npg%s = "%s"\n' "$MAJ" "$BUILD_PGC" > "$PGRX_HOME/config.toml"
fi
"$PGDIR/bin/postgres" --version | grep -q " $MAJ\." || { echo "--pg-dir is not PostgreSQL $MAJ" >&2; exit 2; }
SHAREDIR="$("$PGDIR/bin/pg_config" --sharedir)/extension"
PKGLIBDIR="$("$PGDIR/bin/pg_config" --pkglibdir)"
[ -f "$SHAREDIR/vector.control" ] || { echo "pgvector is not installed in $PGDIR" >&2; exit 2; }
cmp -s "$ENGINE/ontological.control" "$HERE/engine_v1/ontological.control" \
    || { echo "engine_v1/ontological.control differs from engine/ontological.control" >&2; exit 2; }
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }
# Comment lines pgrx writes above each entity: "-- <file>:<line>" and "-- <crate>::<module path>".
strip_locations() { grep -Ev '^-- [^ ]+\.rs:[0-9]+$|^-- [A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)+$'; }

R="$OUT/engine-0-build.out"; : > "$R"
echo "== 1. engine/ as it is (pg$MAJ)"
(cd "$ENGINE" && cargo pgrx schema --release --no-default-features --features "pg$MAJ" "pg$MAJ" \
    -o "$WORK/baseline.sql") > "$WORK/baseline.txt" 2>&1 || { tail -30 "$WORK/baseline.txt"; exit 1; }
strip_ansi < "$WORK/baseline.txt" | grep Discovered | sed 's/^ */engine\/ (ontological): /' | tee -a "$R"

package() { # package <crate> <dir>
    rm -rf "$2"
    (cd "$HERE" && cargo pgrx package -p "$1" --no-default-features --features "pg$MAJ" \
        --pg-config "$BUILD_PGC" --out-dir "$2") > "$2.txt" 2>&1 || { tail -30 "$2.txt"; exit 1; }
    strip_ansi < "$2.txt" | grep Discovered | sed "s/^ */$1: /" | tee -a "$R"
    strip_ansi < "$2.txt" | grep -E '^warning' | sort | uniq -c | sed "s/^/$1 build: /" | tee -a "$R"
}
echo "== 2. engine_v1 shell"
package ontological_k0v1 "$WORK/pkg-v1"
V1SQL="$(find "$WORK/pkg-v1" -name 'ontological--0.1.0.sql')"
{
    echo "--- engine/ vs engine_v1: raw"
    echo "lines: $(wc -l < "$WORK/baseline.sql") vs $(wc -l < "$V1SQL")"
    echo "differing lines: $(diff "$WORK/baseline.sql" "$V1SQL" | grep -c '^[<>]')"
    echo "differing lines that are not '-- ' comments: $(diff "$WORK/baseline.sql" "$V1SQL" | grep '^[<>]' | grep -vc '^[<>] -- ')"
    echo "first differences:"
    diff "$WORK/baseline.sql" "$V1SQL" | head -12
    echo "--- engine/ vs engine_v1: location comments removed"
    if diff <(strip_locations < "$WORK/baseline.sql") <(strip_locations < "$V1SQL") > "$WORK/v1-normalized.diff"; then
        echo "normalized install SQL: identical"
    else
        echo "normalized install SQL: DIFFERENT"; head -40 "$WORK/v1-normalized.diff"
    fi
} | tee -a "$R"

echo "== 3. engine_age shell"
package ontological_age_k0 "$WORK/pkg-age"
AGESQL="$(find "$WORK/pkg-age" -name 'ontological_age--0.1.0.sql')"
python3 - "$AGESQL" "$WORK/ontological_age--0.1.0.sql" <<'PY' | tee -a "$R"
import sys
src, dst = sys.argv[1], sys.argv[2]
text = open(src).read()
begin, end = "/* <begin connected objects> */", "/* </end connected objects> */"
blocks = [b.split(end)[0] for b in text.split(begin)[1:]]
keep = [b for b in blocks if "LANGUAGE c /* Rust */" in b]
open(dst, "w").write("".join(begin + b + end + "\n" for b in keep))
print(f"ontological_age script: {len(blocks)} blocks generated, {len(keep)} pgrx function blocks kept")
PY

echo "== 4. one database"
find "$WORK/pkg-v1" "$WORK/pkg-age" \( -name '*.dylib' -o -name '*.so' \) -exec cp {} "$PKGLIBDIR/" \;
find "$WORK/pkg-v1" "$WORK/pkg-age" -path '*/extension/*' -type f -exec cp {} "$SHAREDIR/" \;
cp "$WORK/ontological_age--0.1.0.sql" "$SHAREDIR/ontological_age--0.1.0.sql"
DATA="$WORK/data"
(exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null && { echo "port $PORT is taken" >&2; exit 2; }
rm -rf "$DATA"
initdb -D "$DATA" -U postgres --auth=trust -E UTF8 --no-locale > "$WORK/initdb.txt" 2>&1 || { cat "$WORK/initdb.txt"; exit 1; }
pg_ctl -D "$DATA" -l "$WORK/postgres.txt" -o "-p $PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=''" -w start >/dev/null \
    || { cat "$WORK/postgres.txt"; exit 1; }
stop_cluster() { pg_ctl -D "$DATA" -m fast -w stop >/dev/null 2>&1; [ "$KEEP" = 1 ] || rm -rf "$DATA"; }
trap stop_cluster EXIT
createdb -h 127.0.0.1 -p "$PORT" -U postgres k0
(cd "$HERE" && psql -X -h 127.0.0.1 -p "$PORT" -U postgres -d k0 --echo-queries -f sql/engine.sql) > "$OUT/engine.out" 2>&1

fails=0
expect() { # expect <file> <grep -E pattern> <what>
    if grep -Eq -- "$2" "$OUT/$1"; then echo "PASS  $3"; else echo "FAIL  $3"; fails=$((fails+1)); fi
}
echo "== expectations (pg$MAJ)"
expect engine-0-build.out '^ontological_k0v1: Discovered 122 SQL entities: 0 schemas \(0 unique\), 119 functions' "engine_v1 ships all 122 entities (119 functions, 3 SQL files) of the library"
expect engine-0-build.out '^ontological_age_k0: Discovered 122 SQL entities' "engine_age ships the same 122"
expect engine-0-build.out "differing lines that are not '-- ' comments: 0" "v1 shell install SQL differs from engine/ only in '-- ' comment lines"
expect engine-0-build.out 'normalized install SQL: identical' "v1 shell install SQL == engine/ once location comments are dropped"
expect engine.out 'ontological +\| 0\.1\.0 +\| public' "ontological (no schema in control) installed into public"
expect engine.out 'ontological_age +\| 0\.1\.0 +\| oga_catalog' "ontological_age installed into oga_catalog"
expect engine.out 'ontological +\| public +\| +119 \| \$libdir/ontological$' "119 C functions in public from \$libdir/ontological"
expect engine.out 'ontological_age +\| oga_catalog +\| +119 \| \$libdir/ontological_age$' "119 C functions in oga_catalog from \$libdir/ontological_age"
expect engine.out '^ +[0-9]+ \| +[0-9]+$' "og_create_graph answers from both objects"
expect engine.out '"name": "Ada"' "oga_catalog.og_cypher reads what public.og_cypher wrote (same v1 code)"
expect engine.out '^ t$' "og_cypher_sql: same compiled SQL from both objects"
expect engine.out 'public +\| og_cypher +\| +1$' "v2 og_cypher_json's unqualified SPI og_cypher(...) ran public.og_cypher (v1)"
expect engine.out '^ontological\.(dylib|so)$' "ontological object mapped in the backend"
expect engine.out '^ontological_age\.(dylib|so)$' "ontological_age object mapped in the same backend"
expect engine.out '^ +0$' "both dropped"
echo "== $fails failed"
exit $(( fails > 0 ))
