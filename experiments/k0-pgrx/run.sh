#!/usr/bin/env bash
# K0 spike, toy crates: build ext_a / ext_b (both over og_shared), install them into a writable
# PostgreSQL tree, start a throwaway cluster, run sql/1..4, stop it and remove its data.
#
#   experiments/k0-pgrx/run.sh --pg 16|17 --pg-dir DIR [--build-pg-config PATH] [--port N] [--work DIR] [--keep]
#
#   --pg-dir           a PostgreSQL install this script may write into (extension files are copied in)
#   --build-pg-config  pg_config whose headers pgrx builds against (default DIR/bin/pg_config; pass the
#                      tree the CARGO_TARGET_DIR cache was built with to reuse it)
#   --port             default 28820 (K0's port)
#   --work             default ${TMPDIR:-/tmp}/k0-pgrx-pg<major>
# Environment: PGRX_HOME (made under --work when unset), CARGO_TARGET_DIR, PATH with cargo-pgrx 0.19.2.
# Writes results/pg<major>/*.out next to this script and prints a PASS/FAIL line per expectation.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MAJ=""; PGDIR=""; BUILD_PGC=""; PORT=28820; WORK=""; KEEP=0
while [ $# -gt 0 ]; do
    case "$1" in
        --pg) MAJ="$2"; shift 2 ;;
        --pg-dir) PGDIR="$2"; shift 2 ;;
        --build-pg-config) BUILD_PGC="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --keep) KEEP=1; shift ;;
        *) sed -n '2,15p' "$0"; exit 2 ;;
    esac
done
[ -n "$MAJ" ] && [ -n "$PGDIR" ] || { sed -n '2,15p' "$0"; exit 2; }
BUILD_PGC="${BUILD_PGC:-$PGDIR/bin/pg_config}"
WORK="${WORK:-${TMPDIR:-/tmp}/k0-pgrx-pg$MAJ}"
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
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

# package <crate> <extra features> <tag>: cargo pgrx package, then copy .so/.dylib, control, SQL into PGDIR
package() {
    local crate="$1" extra="$2" tag="$3" dst="$WORK/pkg/$1-$3"
    rm -rf "$dst"
    (cd "$HERE" && cargo pgrx package -p "$crate" --no-default-features --features "pg$MAJ $extra" \
        --pg-config "$BUILD_PGC" --out-dir "$dst") > "$WORK/build-$crate-$tag.txt" 2>&1 \
        || { tail -30 "$WORK/build-$crate-$tag.txt"; exit 1; }
    strip_ansi < "$WORK/build-$crate-$tag.txt" | grep "Discovered" | sed "s/^ */$crate [$tag]: /" >> "$OUT/0-schema.out"
    find "$dst" \( -name '*.dylib' -o -name '*.so' \) -exec cp {} "$PKGLIBDIR/" \;
    find "$dst" -path '*/extension/*' -type f -exec cp {} "$SHAREDIR/" \;
    cp "$(find "$dst" -name "$crate--*.sql")" "$OUT/install-$crate-$tag.sql"
}
# schema_only <crate> <features> <tag>: entity count of `cargo pgrx schema`, nothing installed
schema_only() {
    local crate="$1" extra="$2" tag="$3"
    (cd "$HERE" && cargo pgrx schema -p "$crate" --release --no-default-features --features "pg$MAJ $extra" \
        "pg$MAJ" -o "$WORK/schema-$crate-$tag.sql") > "$WORK/schema-$crate-$tag.txt" 2>&1 \
        || { tail -30 "$WORK/schema-$crate-$tag.txt"; exit 1; }
    strip_ansi < "$WORK/schema-$crate-$tag.txt" | grep "Discovered" | sed "s/^ */$crate [$tag]: /" >> "$OUT/0-schema.out"
    printf '%s [%s]: og_shared entities in SQL = %s\n' "$crate" "$tag" \
        "$(grep -c '^-- og_shared/src/' "$WORK/schema-$crate-$tag.sql")" >> "$OUT/0-schema.out"
}

DATA="$WORK/data"
psql_run() { (cd "$HERE" && psql -X -h 127.0.0.1 -p "$PORT" -U postgres -d k0 --echo-queries -f "sql/$1.sql") > "$OUT/$1.out" 2>&1; }
stop_cluster() { pg_ctl -D "$DATA" -m fast -w stop >/dev/null 2>&1; [ "$KEEP" = 1 ] || rm -rf "$DATA"; }

: > "$OUT/0-schema.out"
echo "== build (pg$MAJ)"
schema_only ext_b "variant-unlinked" unlinked
schema_only ext_b "variant-extern-only" extern-only
package ext_a "fixed-schema" fixed
package ext_b "fixed-schema" fixed
cat "$OUT/0-schema.out"

echo "== cluster on 127.0.0.1:$PORT"
(exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null && { echo "port $PORT is taken" >&2; exit 2; }
rm -rf "$DATA"
initdb -D "$DATA" -U postgres --auth=trust -E UTF8 --no-locale > "$WORK/initdb.txt" 2>&1 || { cat "$WORK/initdb.txt"; exit 1; }
pg_ctl -D "$DATA" -l "$WORK/postgres.txt" -o "-p $PORT -c listen_addresses=127.0.0.1 -c unix_socket_directories=''" -w start >/dev/null \
    || { cat "$WORK/postgres.txt"; exit 1; }
trap stop_cluster EXIT
createdb -h 127.0.0.1 -p "$PORT" -U postgres k0

psql_run 1-fixed-conflict
package ext_b "" plain          # ext_b without the shared #[pg_schema] module
psql_run 2-coexist
psql_run 3-guc
psql_run 4-order-and-same-schema

# ---------------------------------------------------------------- expectations
fails=0
expect() { # expect <file> <grep -E pattern> <what>
    if grep -Eq -- "$2" "$OUT/$1.out"; then echo "PASS  $3"; else echo "FAIL  $3"; fails=$((fails+1)); fi
}
expect_schema() {
    if grep -Fq -- "$1" "$OUT/0-schema.out"; then echo "PASS  $2"; else echo "FAIL  $2"; fails=$((fails+1)); fi
}
echo "== expectations (pg$MAJ)"
expect_schema "ext_b [unlinked]: og_shared entities in SQL = 0" "a dependency nothing names is not linked: no og_shared entities"
expect_schema "ext_b [extern-only]: og_shared entities in SQL = 7" "'extern crate og_shared as _;' is enough: all 7 og_shared entities (no fixed-schema)"
expect_schema "ext_a [fixed]: Discovered 13 SQL entities: 1 schemas" "ext_a: 13 entities (6 og_shared fns + 2 own, 2+2 sqls, 1 schema)"
expect 1-fixed-conflict 'ERROR: .*(k0_fixed|already exists)' "phase 1: same #[pg_schema] in both -> CREATE EXTENSION ext_b fails"
expect 2-coexist 'ext_a +\| 0\.1\.0 +\| public' "phase 2: ext_a installed into public (no schema in control)"
expect 2-coexist 'ext_b +\| 0\.1\.0 +\| ext_b_s' "phase 2: ext_b installed into ext_b_s (control schema)"
expect 2-coexist 'ext_a \(ext_a\.(dylib|so)\) +\| ext_b \(ext_b\.(dylib|so)\)' "phase 2: public.k0_whoami -> ext_a's object, ext_b_s.k0_whoami -> ext_b's"
expect 2-coexist "ext_a's own k0_edition_local +\| ext_b's own" "phase 2: same-named edition functions stay apart"
expect 2-coexist 'ext_b_s +\| from og_shared extension_sql!, after ext_b bootstrap' "phase 2: shared extension_sql! ran in ext_b_s after ext_b's bootstrap"
expect 2-coexist 'schema=ext_b_s' "phase 2: @extschema@ in a shared SQL file became ext_b_s"
expect 2-coexist 'spi-unqualified -> ext_a \(ext_a\.(dylib|so)\) +\| spi-qualified -> ext_b \(ext_b' "phase 2: unqualified SPI from ext_b reaches ext_a's function (search_path)"
expect 2-coexist '^ext_a\.(dylib|so)$' "phase 2: ext_a's object mapped in the backend"
expect 2-coexist '^ext_b\.(dylib|so)$' "phase 2: ext_b's object mapped in the same backend"
expect 3-guc 'attempt to redefine parameter "k0.shared_probe"' "phase 3: same GUC name defined by the second object -> ERROR"
expect 4-order-and-same-schema 'ext_a +\| public' "phase 4: reverse install order works"
expect 4-order-and-same-schema 'ERROR: .*already exists' "phase 4: both in one schema -> collision"
expect 4-order-and-same-schema '^ +0$' "phase 4: everything dropped"
echo "== $fails failed"
exit $(( fails > 0 ))
