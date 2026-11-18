# AGE probes — is AGE still wrong where the gateway rewrites?

The Bolt gateway's AGE backend (`bolt/src/age/`, `OG_BOLT_BACKEND=age`) rewrites
Cypher around Apache AGE 1.8.0's defects and the Neo4j surface AGE lacks (the
table in `bolt/README.md`, "In front of Apache AGE"). The project rule
(`reports/age-on-supabase/DIRECTION.md` §4.2, rule 2) is that **every rewrite
has a probe**: the plain form, sent straight to AGE *without* the rewrite. When a
newer AGE answers a probe correctly, that rewrite can go.

This directory is that harness. It talks to PostgreSQL only through `psql`
(`ag_catalog.cypher(...)` calls), never through the gateway, and needs nothing
but Python 3 (standard library) and the `psql` binary.

| File | What |
|---|---|
| `probes.json` | the probes, one entry per probe (format below) |
| `run.py` | the runner: `--check`, `--list`, `--dry-run`, or a run against a server |

What each probe stands on — the reports under
`/Users/uengine/uengine-platform/reports/age-on-supabase/` (outside this
repository): `ROBO-ON-AGE.md` §5.1/§5.2 (defects ROBO #1–#14), `SUITES-ON-AGE.md`
§5 (N1–N11), `IMAGE-RESULTS.md` §5.1, §5.11 (①–⑤), `STUDIO-ON-AGE.md` §2.5
(label-table inheritance), and the upstream drafts `upstream/01…18` with their
`repro/` scripts, whose SQL the probes reuse. A probe's `upstream` field names
its draft relative to that folder.

## Kinds and results

| kind | the probe asks | results |
|---|---|---|
| `defect` | does AGE still answer wrong (an error, a wrong value, a crash) where Neo4j answers? | BROKEN · FIXED · CHANGED |
| `surface` | does AGE still lack this piece of Neo4j's Cypher (`datetime()`, `(n:A\|B)`, `FOREACH`, …)? | BROKEN · FIXED · CHANGED |
| `relied_on` | does AGE still behave the way the gateway depends on (label-table inheritance, the forms the rewrites send, `pg_stat_xact_*` counters, agtype text)? | HOLDS · BROKEN |

| status | meaning | what to do |
|---|---|---|
| BROKEN (defect, surface) | AGE answers as it did when the rewrite was written | nothing: the rewrite stays |
| **FIXED** | AGE now gives Neo4j's answer | the rewrite may go — see "When a probe reads FIXED" |
| **CHANGED** | neither the known wrong answer nor Neo4j's | look at `observed` in the JSON: a new symptom (update `broken`), a partial fix, or a probe that needs fixing |
| HOLDS (relied_on) | the behaviour the gateway uses is still there | nothing |
| **BROKEN!** (relied_on) | it is gone: the gateway's rewrite (or the inheritance design) breaks on this AGE | fix the gateway before using this AGE |
| **ERROR** | the harness or the probe's setup failed (connection, permissions, a setup statement) | read `message` |
| SKIPPED | not run: a crasher without `--include-crashers`, `slow` without `--include-slow`, `needs_admin` without `--admin-user`, `disabled`, or `gateway_only` | — |

Exit status: **0** when nothing is FIXED, CHANGED or ERROR and no relied_on probe
is BROKEN; **1** otherwise; **2** for a usage error, a bad `probes.json`, or a
server that cannot be queried.

An asterisk after a probe id in the table means the answer AGE gives today was
**not seen verbatim** in a report (`"source": null` in `probes.json`): it was
written from a description or inferred from how the gateway uses the form. Until
the harness has been run once against the image, a CHANGED or relied-on BROKEN on
such a probe more likely means the expectation needs correcting than that AGE
changed. Every other probe's `broken` answer was recorded on AGE 1.8.0
(`PG15/v1.8.0-rc0`, 88646eb) in the reports, and `source` says where.

## Running it

**Only against a throwaway server.** Some probes kill the backend on purpose
(signal 11, every session on the server is reset) and one fills a fixed-size AGE
table until restart. Never point this at `agesb-db` or any server other people
or agents use.

A throwaway container from the image (Docker must be up; check free disk first):

```bash
# from the repository root
export SECRETS_DIR=$HOME/.agp-secrets            # a private directory (0700)
(cd age-supabase &&
  NAME=agp-probe-db PORT=55450 VOLUME=agp-probe-data NETWORK=agp-probe-net ./scripts/run-db.sh &&
  NAME=agp-probe-db ADMIN_PW_FILE=$SECRETS_DIR/admin.password \
    ROBO_PW_FILE=$SECRETS_DIR/robo.password ./scripts/create-robo-role.sh)
```

The image makes the database `graph` (C collation, `search_path = ag_catalog,
"$user", public`) and `create-robo-role.sh` gives the login role `robo` what it
needs there; `robo` owns the graphs it creates, which AGE needs for new labels.

The usual run (no crashers):

```bash
PSQL=/path/to/psql        # any psql 15+; e.g. the pg16 one in the session scratchpad
python3 tests/age-probes/run.py --host 127.0.0.1 --port 55450 --database graph \
  --user robo --password-file $SECRETS_DIR/robo.password --psql $PSQL \
  --admin-user supabase_admin --admin-password-file $SECRETS_DIR/admin.password \
  --json /tmp/age-probes.json
```

Everything, on the throwaway container only:

```bash
python3 tests/age-probes/run.py --host 127.0.0.1 --port 55450 --database graph \
  --user robo --password-file $SECRETS_DIR/robo.password --psql $PSQL \
  --admin-user supabase_admin --admin-password-file $SECRETS_DIR/admin.password \
  --include-slow --include-crashers --disposable-server --json /tmp/age-probes.json
```

Afterwards: `docker rm -f agp-probe-db && docker volume rm agp-probe-data && docker network rm agp-probe-net`.

Other options:

| option | |
|---|---|
| `--check` | validate `probes.json` (fields, regexes, the safety lints below) and exit |
| `--list` | the probes in run order, with their flags |
| `--dry-run` | print the exact SQL every selected probe would send (setup, probe, cleanup) and the two expected answers; connects to nothing |
| `--only a,b*` | probe ids or globs, repeatable; a `disabled` probe runs only when named exactly |
| `--admin-user`, `--admin-password-file` | a role that may `CREATE DATABASE` (`supabase_admin`), for `needs_admin` probes |
| `--include-slow` | the probes marked `slow` |
| `--include-crashers --disposable-server` | the `kills_backend` probes; the second flag is the acknowledgement, and the first is refused without it |
| `--statement-timeout` | `SET statement_timeout` in every session (default `10s`) |
| `--reconnect-timeout` | seconds to wait for the server after a crash (default 120) |
| `--load-age` | `LOAD 'age'` in every session, for a server that does not preload AGE (superuser only) |
| `--keep-graphs` | leave the probe graphs in place, for debugging |

Passwords come from the files (first line), or from `PGPASSWORD` / `~/.pgpass`
when no file is given; they are never printed or written to the JSON.

### How a probe runs

1. **Setup** (one `psql` session, `ON_ERROR_STOP` on): the probe's `cleanup`
   statements and a `drop_graph` of `agp_<id>` if an earlier run left it (errors
   ignored), then `create_graph('agp_<id>')` and the `setup` statements. A failure
   here is ERROR.
2. **Probe** (a new session): the `probe` statements, as AGE gets them without the
   gateway. Its rows (`psql -A -t`, columns joined by `|`, SQL NULL shown as
   `<null>`, blank lines dropped) or its first error are compared with `broken`
   first, then `fixed`.
3. **Cleanup** (always, even after a failure or a crash): the `cleanup`
   statements, `drop_graph('agp_<id>', true)`, and a check that the graph is gone.

Every session starts with `SET client_min_messages = warning`, `SET search_path =
ag_catalog, "$user", public` (what the gateway's session sets) and `SET
statement_timeout = '10s'`. A probe that needs another search path sets it
itself (the `searchpath_*` probes).

Probes run in file order, then the `slow` ones, then the crashers. After a
crash the runner waits until the server accepts connections again, then cleans
up and goes on with the next probe. If the server does not come back within
`--reconnect-timeout`, the rest are ERROR.

### Reading the JSON

`--json OUT` writes `server` (PostgreSQL and AGE versions, role, database,
search path), the SHA-256 of `probes.json`, and per probe: `status`, `observed`
(`kind`: ok / error / crash / connect / timeout, the normalized `output`, the
`error` text with its DETAIL/HINT lines, the tail of `stderr`), both expected
answers, `message` and `seconds`.

### When a probe reads FIXED

1. Confirm it by hand: `--only <id> --keep-graphs`, look at the graph, and run
   the original Neo4j statement through a gateway built without the rewrite.
2. Remove the rewrite (the code named in `rewrite`), its row in `bolt/README.md`,
   and its unit tests in `bolt/src/age/*.rs`.
3. Turn the probe into a `relied_on` probe whose `fixed` is the now-correct
   answer — the gateway now relies on AGE getting it right — instead of deleting
   it. Keep the `defects` and `upstream` fields.
4. Note it in the upstream draft's tracking (`upstream/INDEX.md`: fixed in which
   version).

## The probe file

```json
{
  "id": "robo01_merge_on_create_then_set",
  "kind": "defect",
  "rewrite": "a SET after a MERGE that has ON CREATE/ON MATCH items → … (split.rs)",
  "defects": ["ROBO #1", "upstream 01"],
  "upstream": "upstream/01-merge-on-set-then-set-not-stored.md",
  "source": "upstream/01 'Actual behavior'",
  "kills_backend": false,
  "slow": false,
  "probe": [
    {"cypher": "MERGE (n:L {k: 1}) ON CREATE SET n.a = 1 SET n.b = 2 RETURN n.b"},
    {"cypher": "MATCH (n:L {k: 1}) RETURN n.a, n.b", "cols": 2}
  ],
  "broken": {"output": "2\n1|<null>"},
  "fixed": {"output": "2\n1|2"}
}
```

| field | |
|---|---|
| `id` | `[a-z][a-z0-9_]{2,40}`; the probe's graph is `agp_<id>` (AGE refuses names under 3 characters) |
| `kind` | `defect`, `surface` or `relied_on` |
| `rewrite` | the gateway rewrite it guards, in the words of `bolt/README.md`'s table (or "(no rewrite) …" for a defect the gateway only lives with) |
| `defects` | the ids used in the reports: `ROBO #1`–`#14`, `SUITES N1`–`N11` (and `A`/`B`/`G` items), `IMAGE-RESULTS §5.11 ①`–`⑤`, `STUDIO G1`–`G8`, `upstream NN` |
| `upstream` | the draft under `reports/age-on-supabase/`, or null |
| `source` | where today's answer was seen; null when it was inferred (the `*` in the table) |
| `kills_backend` | the probe crashes the server process on today's AGE; runs only with `--include-crashers --disposable-server` |
| `slow` | runs only with `--include-slow` |
| `gateway_only` | the rewrite fixes something AGE never sees (e.g. a zone-id DateTime parameter); no SQL, always SKIPPED |
| `needs_admin` | runs as `--admin-user` (e.g. to `CREATE DATABASE`) |
| `graph` | `false`: no `agp_<id>` graph is made (the probe brings its own) |
| `database` | the probe statements run in this database instead of `--database` |
| `disabled` | a reason; the probe runs only when its id is named in `--only` |
| `unordered` | sort the output lines before comparing |
| `setup`, `probe`, `cleanup` | lists of steps (below); `cleanup` runs before and after, errors ignored, before the graph is dropped |
| `broken` | how AGE answers today (not needed for `relied_on`, where it is informational) |
| `fixed` | Neo4j's answer; for `relied_on`, the behaviour the gateway needs |
| `note` | anything else |

A **step** is either SQL text (sent as is; `@graph@` becomes the probe's graph
name; psql meta-commands such as a trailing `\gset` work, and `:'var'` reads a
variable set that way) or a Cypher object:

| key | |
|---|---|
| `cypher` | the Cypher text; becomes `SELECT * FROM ag_catalog.cypher('agp_<id>', $$ … $$) AS (c1 ag_catalog.agtype, …)` |
| `cols` | how many columns it returns (AGE needs the column definition list; default 1, also for a write without RETURN) |
| `params` | a parameter map: the call is `PREPARE`d with `$1` and `EXECUTE`d with this JSON |
| `params_var` | the same, with the JSON taken from a psql variable (`\gset` earlier) |
| `explain` | prefix `EXPLAIN (COSTS OFF)`: the plan, without running the statement |

A **matcher** is an object; every key given must hold. A list of matchers means
any of them.

| key | holds when |
|---|---|
| `"output": "…"` | the statements succeeded and printed exactly this (after normalizing; lines joined by `\n`) |
| `"output_regex": "…"` | the statements succeeded and the output matches (Python `re.search`) |
| `"error": "…"` | a statement failed and its error (with DETAIL/HINT, without the echoed SQL) matches; with `output`, the rows before the error must match too |
| `"crash": true` | the backend died (the connection was lost) |
| `"stderr": "…"` | the whole of stderr matches (e.g. a WARNING), whatever the outcome |

`"error": ".+"` (any error) is used for `surface` probes whose exact refusal was
not recorded: AGE still lacking the form is all they claim.

## Adding a probe when a rewrite is added

1. Reduce the rewrite's trigger to the smallest statement that shows the defect
   on AGE, the way the `repro/` scripts do: its own graph, a few vertices, no
   dependence on other probes.
2. Write the entry: `rewrite` in the words of the new `bolt/README.md` row,
   `defects` and `upstream` as in the reports, `setup` / `probe` steps, `broken` =
   what AGE answers today (run it once with `--only <id>`, and put where you saw
   it in `source`), `fixed` = Neo4j's answer.
3. If the rewrite *sends* a form that AGE must accept (a workaround the gateway
   builds on), add a `relied_on` probe for that form too (the `tgt_*` probes).
4. `python3 tests/age-probes/run.py --check`, then `--dry-run --only <id>`, then a
   run on the throwaway container.

## Safety rules (checked by `--check` and before every run)

- **No probe may exhaust memory or disk.** A destructive variable-length test
  (`*1..30` over a cyclic graph) filled the disk on 2026-11-18 and took Docker
  down. A variable-length pattern in a probe must have an upper bound of at most
  4 hops, unless the statement is only `EXPLAIN`ed (never `EXPLAIN ANALYZE`).
  The VLE probes are a syntax check (`-[:A|B*1..3]-`), small bounded cases, and
  the plan shape behind the memory growth (`robo13_vle_path_on_hash_side`).
- `range()` in a probe spans at most 1000 values; every statement runs under
  `statement_timeout` (10 s; the slow N11 probe sets 120 s for its loop).
- A probe drops only what it made: `drop_graph` / `drop_label` / `DROP
  DATABASE|SCHEMA|TABLE` must name `@graph@` or an `agp_` name.
- A `kills_backend` probe must expect a crash, and runs only with both crasher
  flags.

`suites_n11_graph_version_slots` (slow) makes and drops 130 graphs to show that
AGE 1.8.0's 128 graph-version slots are never freed; afterwards every Cypher write
on that server warns `graph version counter table full` until it restarts. The
harness itself makes about 110 graphs per run, so a second run on the same
server may print that WARNING too; it does not change any result.

## What is not covered

| item | why |
|---|---|
| pg_dump / pg_restore graph OIDs (IMAGE ④, upstream 16) | needs `pg_dump` / `pg_restore` and a second database restored from a dump — beyond `psql`-only probes; the image's `sql/age-after-restore.sql` and `age-supabase/tests/14-dump-roundtrip.sh` cover it |
| the VLE memory exhaustion itself (upstream 12) | destructive by nature; only its plan shape is probed |
| DELETE cost per edge label (upstream 18) | a timing, not an answer; no rewrite depends on it |
| `start_id` / `end_id` variable names | no failing form is known (`upstream/INDEX.md`, "Not reproduced"); `robo04_variable_named_end_id` is kept `disabled` |
| `none()` / `single()` crashing like `any()` / `all()` | not reproduced in the reports; only `any()` and `all()` are probed |
| zone-id DateTime parameters, the `(n:A:B)`-as-either and `elementId` text-scan fixes, `SHOW GRAPHS` / `CREATE GRAPH` | gateway-only: entries with `gateway_only: true`, no SQL |
| SUITES B2–B4, B10, B12 | gateway-side (`CALL {}` handling, error codes): `gateway_only` entries (`gateway_call_subquery_union_and_names`, `gateway_type_error_codes`, `gateway_unknown_procedure`); the forms they send, and the AGE messages the codes are read from, are `relied_on` probes (`tgt_rows_in_every_union_branch`, `tgt_type_error_message`) |
| SUITES B8, B11, B13 | gateway-side (date text check, counters' approximation, `OG_BOLT_AGE_CREATE_GRAPHS`), not AGE behaviour |
