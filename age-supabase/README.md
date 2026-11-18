# age-supabase — Supabase Postgres with Apache AGE

`uengine/supabase-postgres-age:15.8.1.060-age1.8.0` is the local Supabase
database image (`supabase/postgres:15.8.1.060`, PostgreSQL 15.8) with
**unmodified** Apache AGE 1.8.0 (`PG15/v1.8.0-rc0`, commit 88646eb) added:

| What | Where |
|---|---|
| `age.so` | the server's real library dir (`NIX_PGLIBDIR` in the Nix wrapper; `pg_config --pkglibdir` is a different path) |
| `age.control`, `age--*.sql` | `$(pg_config --sharedir)/extension` (what the server reads) and the share dir next to `NIX_PGLIBDIR` |
| preload | `, age` appended to `shared_preload_libraries` in the image's `/etc/postgresql/postgresql.conf` (not `/etc/postgresql-custom`, which is a volume in Compose) |
| install | `/docker-entrypoint-initdb.d/migrations/99999999999999_age.sql`: on a fresh volume, `supabase_admin` runs `CREATE EXTENSION age`, grants `ag_catalog` to `postgres`/`service_role`, and adds an event trigger that refuses `DROP EXTENSION age` (it drops every graph) unless the session sets `age_guard.allow_drop = on` |

Everything else in the Supabase image is unchanged: same roles, settings,
extensions and initdb steps (`tests/supabase-state.sql` diffs a stock
container against this image; the only differences are the four lines above).

## Build

```bash
./build.sh                         # native platform (arm64 on Apple Silicon)
```

1. AGE is compiled in a throwaway `postgres:15.8-bookworm` container
   (`--rm`, `make -j2`, 1500 MB / 2 CPUs) against the exact 15.8 server
   headers pinned from the PGDG archive, with Bison 3.8.2 (AGE pins its
   grammar's conflict counts to 3.8). glibc 2.36 at build time, 2.39 in the
   Supabase image; `age.so` needs only glibc symbols up to 2.17.
2. `docker build` adds one RUN layer (about 1.4 MB) to the Supabase image.
   The artifacts come in through a named build context and a bind mount, so
   no build products stay in the image or in a build stage.

Environment knobs: `SUPABASE_POSTGRES_TAG`, `PG_MINOR` (must match the
Supabase image's PostgreSQL), `AGE_REF` / `AGE_COMMIT`, `AGE_SRC` (use a local
checkout instead of cloning), `IMAGE`, `JOBS`, `KEEP_OUT=1`.

When the Supabase tag changes: set `SUPABASE_POSTGRES_TAG` and `PG_MINOR`
to the new image's PostgreSQL minor version, rebuild, and run `tests/`.

### amd64 (not built here)

- On an amd64 host: `./build.sh` as is — both base images are multi-arch.
- From Apple Silicon through emulation (slow, roughly 10x the native compile):
  `PLATFORM=linux/amd64 IMAGE=uengine/supabase-postgres-age:15.8.1.060-age1.8.0-amd64 ./build.sh`
  (needs the amd64 variant of `supabase/postgres:15.8.1.060`, about 6 GB more disk).
- One tag for both: build each architecture with an `-amd64` / `-arm64`
  suffix, push both, then
  `docker buildx imagetools create -t <repo>:15.8.1.060-age1.8.0 <repo>:15.8.1.060-age1.8.0-amd64 <repo>:15.8.1.060-age1.8.0-arm64`.

## Run

```bash
SECRETS_DIR=/private/dir ./scripts/run-db.sh          # agesb-db on 127.0.0.1:55439, admin password in $SECRETS_DIR/admin.password
NAME=agesb-db ADMIN_PW_FILE=/private/dir/admin.password ROBO_PW_FILE=/private/dir/robo.password \
  ./scripts/create-robo-role.sh                       # login role robo, random password in a 0600 file
```

In Compose, change the `db` service's `image:` line (Lite: `db` and
`migrate`). A volume that was initialised by the stock image does not run the
initdb migration again; run `scripts/install-age.sh` once against it.

## Roles

| Role | For | Grants |
|---|---|---|
| `supabase_admin` | install, admin | superuser (runs the install SQL) |
| `postgres`, `service_role` | Supabase backends | `USAGE` on `ag_catalog`, `SELECT` on `ag_graph`/`ag_label` |
| `robo` | Robo Architect test, Bolt gateway | `USAGE` on `ag_catalog`, `SELECT` on `ag_graph`/`ag_label`, `CONNECT`+`CREATE` on the database. No `BYPASSRLS`, no Supabase role membership |
| `anon`, `authenticated` | browser | nothing |

`robo` creates its own graphs (`ag_catalog.create_graph`), so it owns the graph
schema and every label table. Keep it that way: creating a new label needs
ownership of the graph's base tables (`_ag_label_vertex`/`_ag_label_edge`),
not just `CREATE` on the schema.

## search_path (decision)

AGE's documented setup puts `ag_catalog` on the search path, and 1.8.0 for
PG15 needs it. Without `ag_catalog` on the path, `create_graph`,
any new label, edge patterns, property-map patterns, `WHERE a.x = ...`,
`UNWIND` and `MERGE` fail; only trivial reads work.

So every session that runs Cypher needs `ag_catalog` on its path:

- **Direct connections (the Bolt gateway):** put it in the connection
  options, no code needed:
  `postgres://robo:<pw>@127.0.0.1:55439/postgres?options=-c%20search_path%3Dag_catalog,%22%24user%22,public`
  or key/value `options='-c search_path=ag_catalog,"$user",public'`.
  Verified with the Rust `postgres` 0.19 driver (`tests/rust-agtype`).
- **Through a transaction pooler:** `SET LOCAL search_path = ag_catalog, "$user", public`
  at the start of each transaction.
- **SQL functions exposed through PostgREST:** `SET search_path = ag_catalog, extensions, public, pg_catalog`
  as a function attribute (design 3.6).
- Not used: `ALTER ROLE ... SET search_path` / `ALTER DATABASE ... SET`.
  (For `robo` alone it would be harmless, since PostgREST and pg_graphql do
  not run as `robo`; it is the fallback if a client cannot pass options.)

Keep writing `ag_catalog.cypher(...)` and `ag_catalog.agtype` in SQL anyway.

## Calling cypher() from a gateway

```sql
SELECT * FROM ag_catalog.cypher('graph', $cy0$ MATCH (a:Aggregate {name: $name}) RETURN a, a.version $cy0$, $1)
  AS (a ag_catalog.agtype, version ag_catalog.agtype);
```

- Graph name: a constant. Cypher text: a **dollar-quoted** constant. An
  ordinary `'...'` literal is refused ("a dollar-quoted string constant is
  expected"). Quote client text with a tag that does not occur in it.
- Parameters: one map, passed as the bare `$1` (no cast). Its type is
  inferred as `agtype`. Binary format: one version byte `0x01` + the JSON
  text (`agtype_recv`/`agtype_send`). A `text` parameter with a cast is
  refused ("third argument of cypher function must be a parameter").
- Parameters cannot be used as `CREATE (n:L $props)` or `SET n += $map`
  directly; bind first: `WITH $props AS p CREATE (n:L) SET n = p`,
  `WITH $patch AS p MATCH ... SET n += p`.
- The column list must have exactly as many columns as `RETURN` has items;
  a write without `RETURN` still needs one column. A zero-column list is a
  syntax error.
- Results: agtype text keeps `::vertex`, `::edge`, `::path`, `::numeric` and
  `1` vs `1.0`; this is the best source for Bolt Node / Relationship / Path.
  `::jsonb` works for maps, lists, vertices, edges and paths but **refuses
  top-level integers, floats, booleans and numerics**; use
  `(ag_catalog.agtype_build_list(x)::jsonb) -> 0` for any value (fails only for
  NaN/Infinity). `x::text` is a value cast, not the agtype text (strings lose
  their quotes, `1.0` becomes `1`, edges fail).

## Known limits of AGE 1.8.0 (PG15) seen here

- **TRUNCATE fails in every database that does not have the age extension**
  (`schema "ag_catalog" does not exist`), because the library is preloaded
  cluster-wide and the TRUNCATE hook does not check for the extension
  (master has the check). Workaround: `CREATE EXTENSION age` in those
  databases too (Compose's `_supabase`, any database you create).
- Cypher writes do not fire row or statement triggers on label tables, and do
  not fill column defaults of added columns. CHECK constraints, generated
  columns and table privileges are enforced.
- Added columns on a vertex label: `MERGE` takes a vertex table with exactly
  4 columns for an edge table ("Expecting vertex label, found existing edge
  label"), and `ALTER TABLE ... DROP COLUMN` on a label table is refused.
  Prefer `properties` over added columns.
- `pg_dump`/`pg_restore` keeps the old schema OIDs in `ag_graph.graphid`;
  MATCH then fails with "graph with oid N does not exist". Run
  `sql/age-after-restore.sql` after every restore. Dump the graph schemas
  together with `-e age` (`-n g1 -n g2 -e age`, or a whole-database dump);
  a dump of one graph with `-e age` carries every graph's catalog rows and
  the catalog COPY fails in the target.
- pg_stat_statements groups all Cypher by statement shape (the Cypher text
  is a normalised constant).

## Tests

```bash
cd tests
NAME=agesb-db ADMIN_PW_FILE=... ROBO_PW_FILE=... OUT=/private/dir ./run-all.sh
```

The measured outputs and what they mean are in
`uengine-platform/reports/age-on-supabase/IMAGE-RESULTS.md`.
