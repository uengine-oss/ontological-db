#!/usr/bin/env bash
# pg_dump round trip between two containers of the image.
#   SRC=agesb-db DST=agesb-db2 ADMIN_PW_FILE=... WORK=/private/dir ./14-dump-roundtrip.sh
# Needs a graph robo_t1 (owned by robo) in SRC, e.g. from 10..13.
set -uo pipefail
SRC="${SRC:-agesb-db}"; DST="${DST:-agesb-db2}"
ADMIN_PW_FILE="${ADMIN_PW_FILE:?}"; ROBO_PW_FILE="${ROBO_PW_FILE:?}"; WORK="${WORK:?}"
PW="$(cat "$ADMIN_PW_FILE")"
mkdir -p "$WORK"

q()  { local c="$1"; shift; docker exec -i -e PGPASSWORD="$PW" "$c" psql -X -v ON_ERROR_STOP=0 -U supabase_admin -h 127.0.0.1 "$@" 2>&1; }
qa() { local c="$1" db="$2"; shift 2; docker exec -i -e PGPASSWORD="$PW" "$c" psql -X -At -U supabase_admin -h 127.0.0.1 -d "$db" "$@" 2>&1; }
cy() { # cy <container> <db> <graph> <cypher> : count-style probe as supabase_admin with the AGE path
  qa "$1" "$2" -c "SET search_path = ag_catalog, \"\$user\", public" \
     -c "SELECT * FROM cypher('$3', \$cy\$ $4 \$cy\$) AS (v agtype)"; }
probe() { # probe <container> <db>
  local c="$1" db="$2"
  echo "-- ag_graph (graphid vs the schema's real OID)"
  qa "$c" "$db" -c "SELECT name, graphid, namespace, namespace::oid AS ns_oid, graphid = namespace::oid AS same FROM ag_catalog.ag_graph ORDER BY name"
  echo "-- labels"; qa "$c" "$db" -c "SELECT g.name, l.name, l.id, l.kind FROM ag_catalog.ag_label l JOIN ag_catalog.ag_graph g ON g.graphid = l.graph ORDER BY 1, 3" | tr '\n' ' '; echo
  echo "-- counts in robo_t1"
  echo "vertices: $(cy "$c" "$db" robo_t1 'MATCH (n) RETURN count(n)')  edges: $(cy "$c" "$db" robo_t1 'MATCH ()-[r]->() RETURN count(r)')  vle paths: $(cy "$c" "$db" robo_t1 'MATCH p = (a)-[*1..3]->(b) RETURN count(p)')"
  echo "edge sample: $(cy "$c" "$db" robo_t1 "MATCH (a:Aggregate {name: 'Order'})-[r:HAS]->(b) RETURN [a.name, type(r), b.name]")"
}

echo "### source"
q "$SRC" -d postgres -q -c "SET search_path = ag_catalog, \"\$user\", public" \
  -c "SELECT create_graph('robo_t2')" \
  -c "SELECT * FROM cypher('robo_t2', \$\$ CREATE (:Other {k: 1})-[:REL]->(:Other {k: 2}) \$\$) AS (v agtype)" >/dev/null
probe "$SRC" postgres

dump() { # dump <name> <pg_dump args...>
  local name="$1"; shift
  local t0=$(date +%s)
  docker exec -e PGPASSWORD="$PW" "$SRC" pg_dump -U supabase_admin -h 127.0.0.1 -d postgres -Fc -f "/tmp/$name.dump" "$@" 2>&1 | tail -5
  docker cp -q "$SRC:/tmp/$name.dump" "$WORK/$name.dump"
  docker exec "$SRC" rm -f "/tmp/$name.dump"
  echo "dump $name: $(( $(date +%s) - t0 )) s, $(du -h "$WORK/$name.dump" | cut -f1)"
}
restore() { # restore <name> <target db> <pg_restore args...>
  local name="$1" db="$2"; shift 2
  qa "$DST" postgres -c "DROP DATABASE IF EXISTS $db" -c "CREATE DATABASE $db" >/dev/null
  docker cp -q "$WORK/$name.dump" "$DST:/tmp/$name.dump"
  docker exec -e PGPASSWORD="$PW" "$DST" pg_restore -U supabase_admin -h 127.0.0.1 -d "$db" "$@" "/tmp/$name.dump" > "$WORK/restore-$name.log" 2>&1
  echo "pg_restore $name -> $db: exit $?, $(grep -c 'error:' "$WORK/restore-$name.log") error lines"
  grep 'error:' "$WORK/restore-$name.log" | sed 's/^pg_restore: //' | sort | uniq -c | sort -rn | head -8
  docker exec "$DST" rm -f "/tmp/$name.dump"
}
after_restore_writes() { # writes in the restored copy: sequences, a new label, a new graph
  local c="$1" db="$2"
  echo "-- writes after restore"
  echo "create in existing label: $(cy "$c" "$db" robo_t1 "CREATE (n:Aggregate {name: 'after-restore'}) RETURN id(n)")"
  echo "two highest Aggregate ids: $(qa "$c" "$db" -c "SELECT id FROM robo_t1.\"Aggregate\" ORDER BY id DESC LIMIT 2" | tr '\n' ' ')"
  echo "new label:                $(cy "$c" "$db" robo_t1 "CREATE (n:LabelAfterRestore {k: 1})-[:REL_AFTER_RESTORE]->(n) RETURN labels(n)")"
  echo "new graph:                $(qa "$c" "$db" -c "SET search_path = ag_catalog, \"\$user\", public" -c "SELECT create_graph('g_after_restore')" -c "SELECT * FROM cypher('g_after_restore', \$\$ CREATE (n:X) RETURN count(n) \$\$) AS (v agtype)" -c "SELECT drop_graph('g_after_restore', true)" | tr '\n' ' ')"
  echo "robo can read (owner kept): $(docker exec -e PGPASSWORD="$(cat "$ROBO_PW_FILE")" "$c" psql -X -At -U robo -h 127.0.0.1 -d "$db" -c "SET search_path = ag_catalog, \"\$user\", public" -c "SELECT * FROM cypher('robo_t1', \$\$ MATCH (n) RETURN count(n) \$\$) AS (v agtype)" 2>&1 | tr '\n' ' ')"
}

fix() { # fix <container> <db> : sql/age-after-restore.sql
  docker exec -i -e PGPASSWORD="$PW" "$1" psql -X -q -At -U supabase_admin -h 127.0.0.1 -d "$2" < "$(dirname "$0")/../sql/age-after-restore.sql" 2>&1 | tr '\n' ' '; echo
}
reset_dst_postgres() { # remove graphs restored into the target's postgres database
  qa "$DST" postgres -c "SET search_path = ag_catalog, \"\$user\", public" \
    -c "SELECT drop_graph(name, true) FROM ag_graph" >/dev/null 2>&1
  qa "$DST" postgres -c "DROP SCHEMA IF EXISTS robo_t1 CASCADE" -c "DROP SCHEMA IF EXISTS robo_t2 CASCADE" \
    -c "DELETE FROM ag_catalog.ag_label" -c "DELETE FROM ag_catalog.ag_graph" >/dev/null 2>&1
}

echo; echo "### G1: only the graph schema (-n robo_t1), no extension, into an empty database"
dump g1 -n robo_t1
restore g1 r_g1
echo "cypher in r_g1: $(cy "$DST" r_g1 robo_t1 'MATCH (n) RETURN count(n)' | head -2 | tr '\n' ' ')"
echo "tables in r_g1.robo_t1: $(qa "$DST" r_g1 -c "SELECT count(*) FROM pg_tables WHERE schemaname = 'robo_t1'")"

echo; echo "### G2: one graph plus the extension (-n robo_t1 -e age) while robo_t2 also exists"
dump g2 -n robo_t1 -e age
docker cp -q "$WORK/g2.dump" "$DST:/tmp/g2.list.dump"
echo "TOC entries for the extension and its catalog data:"; docker exec "$DST" pg_restore -l /tmp/g2.list.dump | grep -E 'TABLE DATA ag_catalog|EXTENSION'; docker exec "$DST" rm -f /tmp/g2.list.dump
echo "-- G2a into an empty database (no ag_catalog there)"
restore g2 r_g2
echo "-- G2b into the target's postgres database (age installed by the image)"
reset_dst_postgres
restore_into_postgres() { # restore_into_postgres <name>
  docker cp -q "$WORK/$1.dump" "$DST:/tmp/$1.dump"
  docker exec -e PGPASSWORD="$PW" "$DST" pg_restore -U supabase_admin -h 127.0.0.1 -d postgres "/tmp/$1.dump" > "$WORK/restore-$1-pg.log" 2>&1
  echo "pg_restore $1 -> postgres: exit $?, $(grep -c 'error:' "$WORK/restore-$1-pg.log") error lines"
  grep 'error:' "$WORK/restore-$1-pg.log" | sed 's/^pg_restore: //' | sort | uniq -c | sort -rn | head -5
  docker exec "$DST" rm -f "/tmp/$1.dump"
}
restore_into_postgres g2
probe "$DST" postgres

echo; echo "### G3: every graph schema plus the extension (-n robo_t1 -n robo_t2 -e age) into the target's postgres database"
reset_dst_postgres
dump g3 -n robo_t1 -n robo_t2 -e age
restore_into_postgres g3
probe "$DST" postgres
echo "-- sql/age-after-restore.sql"
fix "$DST" postgres
probe "$DST" postgres
after_restore_writes "$DST" postgres

echo; echo "### F: the whole database (plain pg_dump -Fc as supabase_admin) into an empty database"
dump full
restore full r_full
probe "$DST" r_full
echo "-- sql/age-after-restore.sql"
fix "$DST" r_full
probe "$DST" r_full
after_restore_writes "$DST" r_full

echo; echo "### cleanup in the restore target and the source"
for db in r_g1 r_g2 r_full; do qa "$DST" postgres -c "DROP DATABASE IF EXISTS $db" >/dev/null; done
reset_dst_postgres
q "$SRC" -d postgres -q -c "SET search_path = ag_catalog, \"\$user\", public" -c "SELECT drop_graph('robo_t2', true)" >/dev/null
echo done
