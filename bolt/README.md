# Bolt gateway — spec 011

A Neo4j driver connects here and the application above it does not change.

```python
# was Neo4j
driver = GraphDatabase.driver("bolt://neo4j-host:7687", auth=basic_auth(user, pw))
# now Ontological — same driver, same sessions, same Cypher, same result objects
driver = GraphDatabase.driver("bolt://og-host:7687", auth=basic_auth(pg_role, pg_password))
```

The gateway holds no state: no parser, no planner, no cache, no user store.
Every query it receives goes to `og_cypher()`, so Cypher semantics, compilation,
error messages and transactions are spec 003's — reached through a different
transport, not a second implementation.

## Running it

```bash
cargo build --release

OG_BOLT_PGPORT=28816 OG_BOLT_PGDATABASE=og \
  ./target/release/ontological-bolt
```

| Variable | Default | Meaning |
|---|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` | where to accept Bolt connections |
| `OG_BOLT_PGHOST` | `localhost` | PostgreSQL host |
| `OG_BOLT_PGPORT` | `5432` | PostgreSQL port |
| `OG_BOLT_PGDATABASE` | `og` | PostgreSQL database |
| `OG_BOLT_GRAPH` | `default` | graph used when a session names no database |
| `OG_BOLT_ADVERTISED` | same as `OG_BOLT_LISTEN` | address returned in the routing table |
| `OG_BOLT_STATEMENT_TIMEOUT` | unset: the engine's `og.statement_timeout` (2 min) | every session's statement limit: `300s`, `2min`, `120000` (ms), `0` for none |
| `OG_BOLT_ENGINE_SCHEMA` | unset: engine functions are called unqualified (resolved by `search_path`, which is `public` for the `ontological` extension) | schema to call the engine functions in, e.g. `oga_catalog` for `ontological_age`. Set it explicitly (`public`) when both editions are installed in one database |

## In front of Apache AGE (`OG_BOLT_BACKEND=age`)

The same gateway can stand in front of an unmodified Apache AGE 1.8.0
(PostgreSQL 15, e.g. the Supabase image in `age-supabase/`) instead of the
engine. The default (`ontological`) is unchanged: the five engine calls and
their SQL are the ones above.

| Variable | Default | Meaning |
|---|---|---|
| `OG_BOLT_BACKEND` | `ontological` | `age` to run Cypher through `ag_catalog.cypher()` |
| `OG_BOLT_AGE_CREATE_GRAPHS` | on | make a graph the first time a session names it (as the role that logged in, which then owns it — AGE needs that to add labels). `0`: answer `Neo.ClientError.Database.DatabaseNotFound` ("… make it with CREATE GRAPH x"); `SHOW GRAPHS` and `CREATE GRAPH x` work either way, whatever database the session names |
| `OG_BOLT_AGE_LOAD` | off | `LOAD 'age'` at login, for a server that does not preload AGE (superuser only) |
| `OG_BOLT_AGE_TRACE` | off | log every SQL statement sent to AGE and its time, and each statement's plan (cypher, steps, reach, ddl …), to stderr |
| `OG_BOLT_AGE_COUNTERS` | on | read `summary.counters` (two short SQL statements around every write); off: writes report no counters |

A Bolt database is the AGE graph of the same name, except a name of fewer
than 3 characters, which AGE refuses ("graph name is invalid", SUITES N10;
Neo4j refuses those too, but Robo's launcher makes 2-character projects,
`^[a-z][a-z0-9_]{1,39}$`): `ab` is kept in the AGE graph `_og_ab`. `SHOW
GRAPHS` lists it as `ab`, `CREATE GRAPH ab` makes `_og_ab`, and every
statement of a session on `ab` runs there (`mod.rs` `age_graph_name`). No
client name can land there by itself: Neo4j's database names start with a
letter, and `_og_` followed by one or two characters is refused as a database
name. Names of 3 characters or more are the graph of the same name, as
before. `CREATE GRAPH` of a graph that is there already answers
`Neo.ClientError.Database.ExistingDatabaseFound` (AGE gives it 3F000, the
code of a missing schema). An error message from AGE itself names the graph
as AGE has it (`_og_ab`).

The session sets `search_path = ag_catalog, "$user", public` (AGE's generated
SQL needs it). `HELLO` answers `Neo4j/4.4.0 (age-bolt/<version>)`, without
"ontolog", so clients that send Ontological's fabric statements to an
Ontological gateway (Studio, robo-data-catalog, robo-data-analyzer) do not.

What happens to a statement (`src/age/`):

- it is parsed with the engine's own parser (`../cypher-syntax`, the engine's
  lexer, parser and AST built without pgrx), which gives the RETURN columns
  and read/write exactly as `og_cypher_columns()` / `og_cypher_check()` do;
- it is printed back as AGE's Cypher inside
  `SELECT * FROM ag_catalog.cypher('<graph>', $og$…$og$, $1) AS (c0 agtype, …)`,
  the parameters going as one agtype map in `$1` (binary: `0x01` + JSON);
- results are read as agtype text: `::vertex` → `Node`, `::edge` →
  `Relationship`, `::path` → `Path`, `1` and `1.0` kept apart. A node's id
  is AGE's graphid, and the element id a Bolt 4.4 driver derives from it is
  `toString(id(n))`, which `elementId(n)` is rewritten to.

Rewrites (each found by running Robo Architect's queries against AGE; see
`cypher.rs`, `split.rs` and `reach.rs` for the measurement behind each):

| Neo4j | sent to AGE |
|---|---|
| `datetime()`, `date()`; `datetime($p)`; `datetime() - duration({minutes: $m})` | the statement's clock, `$p` written as PostgreSQL writes a timestamptz, the shifted instant — as ISO 8601 text in parameters |
| a DateTime parameter with a zone id (Bolt `0x66`; SUITES B9: the zone was dropped and the wall time read as UTC, nine hours off for Asia/Seoul) | its instant, asked of PostgreSQL's zone database before the statement (`timezone(zone, local::timestamp)`, one query per distinct value — `run.rs` `age_params`), as the canonical UTC text the other date-times take. A DateTime with an offset already says its instant and is left alone |
| `randomUUID()` | `toString(pg_catalog.concat(pg_catalog.gen_random_uuid()))` |
| `id(x) IN list` (AGE: `id(n) IN $param` kills the backend, signal 11; `id(n) IN [1, 2]` fails "not a common type" — ROBO 11.4) | `id(coalesce(x)) IN list` |
| `elementId(x) = e`, `e = elementId(x)` (STUDIO G4: `toString(id(…)) = e` reads every row, 28 ms → 2 ms at 25k vertices) | `id(x) = $__og_eid_p` / `id(x) = 12` when the parameter or literal is an integer's text; `id(x) = toInteger(e) AND toString(id(coalesce(x))) = e` for an expression (the index finds it, the text decides); the text comparison for anything else. `elementId(x) IN list` stays a text comparison (`id(x) IN` is the crash above) |
| `n:L` in an expression; `(n:A\|B)`, `-[:A\|B]-` | `label(n) = 'L'`, or `label(n) IN ['L', the labels whose tables inherit L's]` when there are any (*Label hierarchy* below); the choice moved into the clause's WHERE — for a node, `label(n) IN [A, B, the labels below them]` |
| `(n:A:B)` in MATCH / OPTIONAL MATCH / EXISTS — both labels (SUITES B1: was read as either, `MATCH (n:Lamp:Part)` counted Lamps and Parts; STUDIO G2: `(n:_Entity:Customer)` became a scan of every vertex, 6–10 ms at 25k, where one table takes 1 ms) | `(n:B)` — the last label's table, which AGE scans together with the tables inheriting it — with nothing more when every one of those tables has A too (`(n:_Entity:Customer)` once `Customer` inherits `_Entity`), `label(n) IN [those that do]` when some do, `false` when none does; `(n:A)` for `(n:A:A)` (`cypher.rs` `node_label_test`) |
| `CREATE` / `MERGE (n:A:B:C {…})` (STUDIO G1, ANALYZER §5.1: refused before — the analyzer's every node write, Studio's entity, ingest and publish writes) | `(n:C {…})`, and before it, in its transaction, the label tables that are missing (`create_vlabel`) and the inheritance C's table lacks (`ALTER TABLE "g"."C" INHERIT "g"."A"`) — *Label hierarchy* below (`hierarchy.rs`) |
| `labels(x)` (STUDIO G3: `["Customer"]` where Neo4j has `["Customer", "_Entity"]`; `$class IN labels(n)` found nothing) | `(CASE label(x) WHEN 'Customer' THEN ['Customer', '_Entity'] … ELSE labels(x) END)`, a branch for each label whose table inherits others, nearest first (`labels(n)[0]` is the specific label); as written when no table inherits |
| `any/all/none/single(x IN l WHERE p)` | `size([x IN l WHERE p]) > 0` … (AGE's own crash the backend when `p` reads a matched node) |
| `n {.a, .b}` | `CASE WHEN n IS NULL THEN null ELSE {a: n.a, b: n.b} END` (Neo4j's answer for null and for an absent key) |
| `n {.*, k: e, .p}` (ANALYZER §5.4: AGE builds `age_properties(n) + agtype_build_map_nonull(…)` — `n {.*, embedding: null}` kept the stored `embedding`, `n {.*, id: 'x'}` came back as `{id: 'x'}` alone) | `CASE WHEN n IS NULL THEN null ELSE (properties(n) - ['k', 'p']) + {k: e, p: n.p} END`: the entries' keys taken out of the properties first (`agtype - [keys]`), then the entries as a map literal, which keeps a null entry (`cypher.rs` `all_properties_and`). `n {.*}` alone is sent as it is Only when an entry can be null (an expression, `null`, or `.p`): with entries that are non-null literals AGE is right (probe `analyzer_map_projection_all_key_override`, HOLDS on 1.8.0) and the projection goes to AGE as written. |
| `coalesce(a, b)` over nodes | `coalesce(head([a]), head([b]))` |
| `SET n += $map`; `SET r = item.props`, `SET n += item.props` (ANALYZER §5.3: AGE takes a map literal, a function call or a variable there — "SET clause expects a map" for a parameter or a property access) | `WITH …, $map AS __og_p1 SET n += __og_p1`, `WITH …, item.props AS __og_setv1 SET r = __og_setv1` (`cypher.rs` `bind_map_params`); left as written when the value reads what a MERGE itself binds |
| a MERGE pattern property that is null (SUITES N7: AGE makes an element without the key, a new one every time; Neo4j refuses) | known now (a literal, a parameter given, a field of one): refused with Neo4j's `SemanticError` "Cannot merge the following node because of null property value for 'key': (:W {key: null})" ("…relationship…"). Computed (`r.missing`): sent as `CASE WHEN e IS NULL THEN 1 / CASE WHEN e IS NULL THEN 0 ELSE 1 END ELSE e END`, and AGE's "division by zero" from such a statement is answered with that error (`cypher.rs` `guard_merge_nulls`, `mod.rs` `null_merge_refusal`) |
| an aggregation next to a list comprehension | three projections: lists, aggregates, the rest |
| variables named `id`, `properties`, `start_id`, `end_id` | renamed (they clash with the label tables' columns) |
| a leading `OPTIONAL MATCH` | `WITH 1 AS …` in front (AGE returns no row when nothing matches) |
| `ORDER BY m.k` of a projected map `m` | the key read from where the map got it |
| `RETURN *` | the variables in scope, sorted |
| `FOREACH`, `CALL { … }`, `SET n:L` / `REMOVE n:L`, a SET after a MERGE that has ON CREATE/ON MATCH items | cut into several statements in one transaction, the rows carried between them as a parameter (vertices and edges by id) — AGE has no FOREACH or CALL {}, keeps one label per vertex, and drops a SET that follows such a MERGE |
| `CALL { … UNION [ALL] … }` (SUITES B2: "could not find rte for s" — the rows were brought in on the first branch only) | every branch gets them: `UNWIND $__og_rows AS __og_row OPTIONAL MATCH (s:L) WHERE id(s) = __og_row.s WITH __og_row, s <branch> UNION ALL UNWIND $__og_rows …` |
| `CALL { … }` names (SUITES B3: answered where Neo4j refuses) | checked before anything runs, with Neo4j's `SyntaxError` and wording: a name it returns that is bound outside ("Variable `s` already declared in outer scope"), a name it imports — `CALL (nope)`, a leading `WITH nope` — that is not bound ("Variable `nope` not defined") |
| a FOREACH, `CALL { … }` or label change inside a FOREACH or `CALL { … }` body (SUITES B4: refused) | the body cut into steps of its own and run on the rows the outer step gives it (`split.rs` `plan_in` / `run_on`): a FOREACH's on one row per row and element, made by one statement (`… UNWIND list AS x RETURN <scope>, x`), a CALL's once per row. A vertex several such rows hold changes table once |
| a WHERE after a SET / REMOVE whose part reads only what was there at the SET (SUITES N4: AGE evaluates it below the SET — `MATCH (s:V) SET s.v = s.k * 10 WITH s WHERE s.v > 10` filters on the old value, and the rows it drops are never written) | cut before the WHERE's clause (`split.rs` `Cut`); a WHERE on what came after the SET (`OPTIONAL MATCH (a …) WHERE a IS NOT NULL`, an UNWIND's variable) stays |
| a MATCH / OPTIONAL MATCH after a CREATE / MERGE in the same part whose new elements can see what was written — a label or type written there, a node without a label, a relationship without a type (SUITES N3: `UNWIND [1,2,3] AS i CREATE (:V {k: i}) WITH count(*) AS made MATCH (t:V)` sees 1 of 3) | cut before the MATCH. `MERGE (a:X …) WITH a MATCH (b:Y …)` stays one statement |
| a MERGE whose ON CREATE / ON MATCH items read what it merges (SUITES N1, and #7 for a relationship: "attribute 1 of type record has wrong type", "binary container must be a VLE vpc") | the MERGE with `ON CREATE SET m.__og_c = true` on its first relationship (given a variable) or its node; then the ON MATCH items on the rows that did not create (`NOT first-to-reach OR m.__og_c IS NULL`), the ON CREATE items and `m.__og_c = null` on those that did. Items that read only other variables leave the MERGE as it is |
| a step that writes (SET / REMOVE first in a part, or N1's items) on rows holding the same element more than once (SUITES N2: `UNWIND [1,1,1] AS i MERGE (t:U {k: i}) ON MATCH SET t.hits = coalesce(t.hits, 0) + 1` stores 1; each row's SET starts from the element as it was before the statement, and a final SET skips a tuple already updated in its command) | run in waves — no element twice in a wave, each element's rows in order (`split.rs` `waves`); one statement when the rows' elements are distinct. The rows the statement returns afterwards show the final values, where Neo4j's would show each row's own |
| `CREATE CONSTRAINT … IS UNIQUE`, `CREATE INDEX` | unique / plain expression indexes on the label's table (made if missing); `IS NOT NULL`, full-text and vector are accepted and not enforced |
| `SHOW GRAPHS`, `CREATE GRAPH g` | answered from `ag_catalog` |
| procedures and `SHOW INDEXES` / `SHOW CONSTRAINTS` (SUITES B5–B7, STUDIO G5a–G5d, ANALYZER §5.2: refused, "CALL x() inside a query is not supported") | answered by the gateway (*Procedures and SHOW* below): alone, from the catalog; with `YIELD … WITH … WHERE … RETURN … ORDER BY` after it, the rows as a parameter and the rest in Cypher — `UNWIND $__og_proc1 AS label …` |
| `CALL apoc.neighbors.tohop(n, '>', d) YIELD node AS m` (STUDIO G5c) | `MATCH (n)-[*1..d]->(m) WHERE id(m) <> id(n) WITH DISTINCT …, m`, `d` a literal or a parameter; with no type or several and over 3 hops, walked by `reach.rs` (the MATCH is followed by DISTINCT) |
| an unknown procedure (SUITES B12: was `SyntaxError`) | `Neo.ClientError.Procedure.ProcedureNotFound`, Neo4j's wording |
| a variable-length relationship without exactly one type over 3 hops (`-[:A\|B\|…*1..30]-`, `<-[*1..30]-`) where only the nodes it reaches matter: in `EXISTS { }` between two bound nodes, or as the only pattern of a MATCH followed by `DISTINCT`, `DISTINCT` aggregates (and `min`/`max`) or `LIMIT 1` | walked by the gateway (`reach.rs`): the start nodes' ids from the clauses before it, a breadth-first walk with one SQL statement per hop on the label tables' `start_id`/`end_id`, then the statement with the pattern replaced by `id(t) IN $reached[id(s)]`. Neo4j's trail semantics: the start node counts as reached only over a closed trail (a cycle through it) within the bound |

### Rewrite register

Every rewrite above, numbered, with the AGE defect or missing Neo4j surface it is for, its upstream draft (`uengine-platform/reports/age-on-supabase/upstream/NN-*.md`) and its probes in `tests/age-probes/` — the plain form sent straight to AGE. **BROKEN**: AGE still gets it wrong, the rewrite stays. **HOLDS** (kind `relied_on`): an AGE behaviour the rewrite depends on still holds. A probe turning FIXED on a newer AGE means that rewrite can go; a relied-on probe turning BROKEN means the rewrite breaks. Generated by `python3 tests/age-probes/register.py --json <run.json>`; the statuses are the run of 2026-11-18 on AGE 1.8.0 (PG15) in a throwaway container (82 BROKEN, 48 HOLDS, 8 skipped: gateway-only or disabled, 0 FIXED/CHANGED).

| # | rewrite (what AGE gets wrong or lacks → what the gateway sends) | kind | defect ids | upstream draft | probes |
|---|---|---|---|---|---|
| R01 | a SET after a MERGE that has ON CREATE/ON MATCH items → cut into several statements in one transaction: the MERGE ends one, the SET runs as the next (split.rs) | defect | ROBO #1, upstream 01 | 01 | `robo01_merge_on_create_then_set` BROKEN |
| R02 | a REMOVE after a MERGE whose ON MATCH branch has items → cut after the MERGE (split.rs) | defect | ROBO #1, upstream 01 | 01 | `robo01_merge_on_match_then_remove` BROKEN |
| R03 | any/all/none/single(x IN l WHERE p) → size([x IN l WHERE p]) > 0 (= 0, = 1; all: NOT p counted = 0) (cypher.rs ListPred) | defect | ROBO #2, upstream 02 | 02 | `robo02_any_reads_matched_vertex` BROKEN |
| R04 | any/all/none/single(x IN l WHERE p) → size([x IN l WHERE p]) … (cypher.rs ListPred) | defect | ROBO #2, upstream 02 | 02 | `robo02_all_reads_matched_vertex` BROKEN |
| R05 | an aggregation next to a list comprehension → three projections: lists, aggregates, the rest | defect | ROBO #3, upstream 03 | 03 | `robo03_aggregate_and_comprehension` BROKEN, `robo03_group_key_and_comprehension` BROKEN |
| R06 | variables named id, properties, start_id, end_id → renamed (they clash with the label tables' columns) | defect | ROBO #4, upstream 04 | 04 | `robo04_variable_named_id` BROKEN |
| R07 | variables named id, properties, start_id, end_id → renamed | defect | ROBO #4, upstream 04 | 04 | `robo04_variable_named_properties` BROKEN |
| R08 | variables named id, properties, start_id, end_id → renamed (start_id / end_id: defensively) | defect | ROBO #4 | 04 | `robo04_variable_named_end_id` SKIPPED |
| R09 | a leading OPTIONAL MATCH → WITH 1 AS … in front (AGE returns no row when nothing matches) | defect | ROBO #5, upstream 05 | 05 | `robo05_leading_optional_match` BROKEN |
| R10 | n {.a, .b} → CASE WHEN n IS NULL THEN null ELSE {a: n.a, b: n.b} END | defect, relied_on | ROBO #6, upstream 06 | 06 | `robo06_map_projection_absent_key` BROKEN, `tgt_map_projection_as_case` HOLDS |
| R11 | n {.a, .b} → CASE WHEN n IS NULL THEN null ELSE {…} END | defect | ROBO #6, upstream 06 | 06 | `robo06_map_projection_of_null` BROKEN |
| R12 | a MERGE whose ON CREATE / ON MATCH items read what it merges → the MERGE marks what it creates, the items run after it (split.rs N1 / #7) | defect | ROBO #7, upstream 07 | 07 | `robo07_merge_rel_on_match_reads_itself` BROKEN |
| R13 | ORDER BY m.k of a projected map m → the key read from where the map got it | defect | ROBO #8, upstream 08 | 08 | `robo08_order_by_projection_alias` BROKEN |
| R14 | toString(id(n)) → toString(id(coalesce(n))); elementId(x) → toString(id(coalesce(x))) | defect | ROBO #9, upstream 09 | 09 | `robo09_tostring_of_id` BROKEN |
| R15 | toInteger(id(n)) → toInteger(id(coalesce(n))) | defect | ROBO #9, upstream 09 | 09 | `robo09_tointeger_of_id` BROKEN |
| R16 | id(n) used as a number → id(coalesce(n)) (README: id(x) IN list → id(coalesce(x)) IN list) | defect | ROBO #9, upstream 09 | 09 | `robo09_id_arithmetic` BROKEN |
| R17 | id(x) IN list → id(coalesce(x)) IN list (AGE: id(n) IN [1, 2] fails 'not a common type' — ROBO 11.4) | defect | ROBO #9, upstream 09 | 09 | `robo09_id_in_literal_list` BROKEN |
| R18 | id(x) IN list → id(coalesce(x)) IN list (AGE: id(n) IN $param kills the backend, signal 11) | defect | ROBO #9, upstream 09, ROBO 11.4 | 09 | `robo09_id_in_parameter` BROKEN |
| R19 | id(x) IN list → id(coalesce(x)) IN list (the crash without a parameter) | defect | ROBO #9, upstream 09 | 09 | `robo09_id_in_variable` BROKEN |
| R20 | coalesce(a, b) over nodes → coalesce(head([a]), head([b])) | defect, relied_on | ROBO #10, upstream 10 | 10 | `robo10_coalesce_over_vertices` BROKEN, `tgt_coalesce_through_head` HOLDS |
| R21 | randomUUID() → toString(pg_catalog.concat(pg_catalog.gen_random_uuid())) — not a reduce() over rand(), which AGE refuses inside ON CREATE / ON MATCH SET | defect | ROBO #11, upstream 11 | 11 | `robo11_reduce_in_on_create_set` BROKEN |
| R22 | randomUUID() → toString(pg_catalog.concat(pg_catalog.gen_random_uuid())) | surface | ROBO #11 | — | `robo11_random_uuid` BROKEN |
| R23 | exists(pattern) that names a variable of its own → EXISTS { MATCH … } | defect | ROBO #12 | — | `robo12_exists_names_new_variable` BROKEN |
| R24 | a variable-length relationship without exactly one type over 3 hops → walked by the gateway (reach.rs, breadth first, one SQL statement per hop); up to 3 hops → -[v*1..n]- and the types tested on v | surface | ROBO #13, ROBO 11.4, upstream 12 (context) | 12 | `robo13_vle_with_two_types` BROKEN |
| R25 | the refusal of long multi-type / untyped VLEs whose trails matter, and the walk in reach.rs: AGE keeps every enumerated path of a VLE with a path variable on the hash side of a join (memory grows with the paths) | defect | ROBO #13, upstream 12 | 12 | `robo13_vle_path_on_hash_side` BROKEN |
| R26 | SET n:L → the vertex moved to the label's table by SQL in the same transaction (split.rs relabel); a vertex keeps one label | surface | ROBO #14, SUITES A1 | — | `robo14_set_label` BROKEN |
| R27 | REMOVE n:L → the vertex moved to _ag_label_vertex by SQL (split.rs relabel) | surface | ROBO #14 | — | `robo14_remove_label` BROKEN |
| R28 | refused by name: a node with two labels (CREATE / MERGE (:A:B)); with label-table inheritance the gateway is to store (:_Entity:X) as X inheriting _Entity (DIRECTION §4.2 rule 5, STUDIO G1) | surface | ROBO #14, SUITES A1, IMAGE-RESULTS §5.10 | — | `multi_label_create` BROKEN |
| R29 | datetime(), date() → the statement's clock, as ISO 8601 text in a parameter | surface | ROBO §5.1 (surface), SUITES A11 | — | `surface_datetime_now` BROKEN |
| R30 | date() → the statement's date, as ISO 8601 text in a parameter | surface | ROBO §5.1 (surface), SUITES A11 | — | `surface_date_today` BROKEN |
| R31 | datetime($p) → $p written as PostgreSQL writes a timestamptz | surface | ROBO §5.1 (surface) | — | `surface_datetime_of_parameter` BROKEN |
| R32 | datetime() - duration({minutes: $m}) → the shifted instant, worked out by the gateway | surface | ROBO §5.1 (surface), SUITES A11 | — | `surface_duration` BROKEN |
| R33 | a DateTime parameter with a zone id (Bolt 0x66) → its instant, asked of PostgreSQL's zone database (run.rs age_params); the gateway used to drop the zone and read the wall time as UTC | gateway-only | SUITES B9, SUITES G13 | — | `gateway_zone_id_datetime` SKIPPED |
| R34 | elementId(x) → toString(id(coalesce(x))); elementId(x) = e → id(x) = <e as an integer> | surface | IMAGE-RESULTS §5.10, STUDIO G4 | — | `surface_element_id` BROKEN |
| R35 | elementId(x) = e → id(x) = <integer> instead of the gateway's earlier toString(id(…)) = e, which read every row (STUDIO G4: 28 ms → 2 ms at 25k vertices) | gateway-only | STUDIO G4 | — | `gateway_element_id_text_scan` SKIPPED |
| R36 | n:L in an expression → label(n) = 'L' | surface | ROBO §5.1 (surface) | — | `surface_label_predicate` BROKEN |
| R37 | (n:A\|B) → (n) and label(n) IN ['A', 'B'] in the clause's WHERE | surface | ROBO §5.1 (surface), IMAGE-RESULTS §5.10 | — | `surface_label_choice` BROKEN |
| R38 | -[:A\|B]- → -[v]- and type(v) IN ['A', 'B'] in the clause's WHERE | surface | ROBO §5.1 (surface) | — | `surface_type_choice` BROKEN |
| R39 | (n:A:B) in MATCH / OPTIONAL MATCH / EXISTS → (n:A) WHERE false when A ≠ B (a vertex has one label), (n:A) for (n:A:A); decided in cypher.rs node_label_test, which label inheritance will change | surface | SUITES B1, STUDIO G2 | — | `surface_two_labels_in_match` BROKEN |
| R40 | (n:A:B) means both labels, (n:A\|B) either — the gateway had read both as either (label(n) IN [...]); MATCH (n:Lamp:Part) counted Lamps and Parts | gateway-only | SUITES B1, STUDIO G2 | — | `gateway_two_labels_read_as_either` SKIPPED |
| R41 | SET n += $map → WITH …, $map AS p SET n += p | relied_on, surface | ROBO §5.1 (surface), IMAGE-RESULTS §5.3 | — | `surface_set_map_parameter` BROKEN, `tgt_set_map_bound_by_with` HOLDS |
| R42 | RETURN * → the variables in scope, sorted (Neo4j's column order; the gateway also needs the count for the column definition list) | surface | ROBO §5.1 (surface) | — | `surface_return_star_columns` BROKEN |
| R43 | FOREACH → the writes for every row and element, as UNWIND $rows … UNWIND list AS x (split.rs) | surface | ROBO §7 (R3), apache/age #2381 | — | `surface_foreach` BROKEN |
| R44 | CALL { … } → the body once per row, the rows carried as a parameter (split.rs) | surface | ROBO §7 (R3), apache/age #1690 | — | `surface_call_subquery` BROKEN |
| R45 | CREATE CONSTRAINT … IS UNIQUE → a unique expression index on the label's table (ddl.rs) | surface | ROBO §5.1 (surface), apache/age #1635 | — | `surface_create_constraint` BROKEN |
| R46 | CREATE INDEX → a plain expression index on the label's table (ddl.rs) | surface | ROBO §5.1 (surface), apache/age #1635 | — | `surface_create_index` BROKEN |
| R47 | CALL db.labels() → answered from ag_catalog.ag_label; with YIELD … WITH … WHERE … RETURN after it, the rows as a parameter: UNWIND $__og_procN AS label … (procs.rs, SUITES B5) | surface | ROBO §5.1 (surface), IMAGE-RESULTS §5.10 | — | `surface_db_labels` BROKEN |
| R48 | CALL db.propertyKeys() → keys() of every node and relationship, read through Cypher (procs.rs; it was a SQL read of the label tables); db.relationshipTypes() → ag_catalog.ag_label | surface | ROBO §5.1 (surface) | — | `surface_db_property_keys` BROKEN |
| R49 | SHOW GRAPHS, CREATE GRAPH g → answered from ag_catalog / ag_catalog.create_graph() | gateway-only | ROBO §5.1 (surface) | — | `gateway_show_and_create_graph` SKIPPED |
| R50 | refused: a pattern comprehension or COLLECT { … } (cypher.rs) | surface | SUITES A12, SUITES G21 | — | `surface_pattern_comprehension` BROKEN |
| R51 | operator precedence: every operand that is not atomic is parenthesised (AGE binds IN tighter than arithmetic) | surface | ROBO §5.1 (precedence) | — | `precedence_in_below_plus` BROKEN |
| R52 | operator precedence: … AGE binds IS NULL tighter than arithmetic | surface | ROBO §5.1 (precedence) | — | `precedence_is_null_below_plus` BROKEN |
| R53 | operator precedence: … AGE binds STARTS WITH / CONTAINS / =~ tighter than arithmetic | surface | ROBO §5.1 (precedence) | — | `precedence_starts_with_below_plus` BROKEN |
| R54 | operator precedence: … AGE binds XOR tighter than AND | surface | ROBO §5.1 (precedence) | — | `precedence_xor_above_and` BROKEN |
| R55 | a MERGE whose ON CREATE / ON MATCH items read what it merges → the MERGE with ON CREATE SET m.__og_c = true, then the ON MATCH items on the rows that did not create, the ON CREATE items and m.__og_c = null on those that did (split.rs) | defect | SUITES N1, SUITES A3, upstream 07 | 07 | `suites_n1_on_match_reads_merged` BROKEN, `suites_n1_with_then_merge_reads_merged` BROKEN, `suites_n1_on_create_reads_merged` BROKEN |
| R56 | a step that writes on rows holding the same element more than once → run in waves, no element twice in a wave (split.rs waves) | defect | SUITES N2, SUITES A4 | — | `suites_n2_same_element_twice` BROKEN |
| R57 | a MATCH / OPTIONAL MATCH after a CREATE / MERGE in the same part that can see what was written → cut before the MATCH (split.rs Cut) | defect | SUITES N3, SUITES A5 | — | `suites_n3_match_after_create` BROKEN |
| R58 | a WHERE after a SET / REMOVE whose part reads only what was there at the SET → cut before the WHERE's clause (split.rs Cut) | defect | SUITES N4, SUITES A6 | — | `suites_n4_where_on_set_property` BROKEN |
| R59 | a WHERE after a SET → cut before the WHERE's clause (split.rs Cut): the rows the WHERE drops are never written | defect | SUITES N4 | — | `suites_n4_filtered_rows_not_written` BROKEN |
| R60 | (no rewrite yet) size() of a string counts bytes; the gateway could rewrite it for constants and parameters only (SUITES G16) | defect | SUITES N5, SUITES A7 | — | `suites_n5_size_of_string` BROKEN |
| R61 | (no rewrite yet) a property of a function result: coalesce(a, b).k → CASE … (SUITES G17) | defect | SUITES N6, SUITES A8 | — | `suites_n6_property_of_function_result` BROKEN |
| R62 | (no rewrite yet) vector.similarity.cosine(…) as a list expression (SUITES G18) | defect | SUITES N6, SUITES A8 | — | `suites_n6_three_part_function_name` BROKEN |
| R63 | a MERGE pattern property that is null → refused with Neo4j's SemanticError when known; computed: guarded by a division by zero that the gateway answers with that error (cypher.rs guard_merge_nulls) | defect | SUITES N7, SUITES A9 | — | `suites_n7_merge_null_key` BROKEN |
| R64 | (no rewrite of the statement) SET on a map, not an element: AGE's internal message (XX000 container is not an agtype array) is answered as Neo.ClientError.Statement.TypeError (mod.rs error_code, SUITES G14) | defect | SUITES N8, SUITES B12 | — | `suites_n8_set_on_a_map` BROKEN |
| R65 | (no rewrite yet) toInteger(true) → CASE WHEN x = true THEN 1 … (SUITES G19) | defect | SUITES N9, SUITES A10 | — | `suites_n9_tointeger_of_boolean` BROKEN |
| R66 | a Bolt database name under three characters → the AGE graph _og_<name> (mod.rs age_graph_name; SHOW GRAPHS shows it as <name>, and _og_ with one or two characters is refused as a database name) — SUITES G27, Robo's launcher makes two-character projects | defect | SUITES N10, SUITES A14 | — | `suites_n10_short_graph_name` BROKEN |
| R67 | (no rewrite) AGE's graph version counter table has 128 slots that drop_graph does not free; after that every Cypher write warns and the VLE cache falls back to snapshots | defect | SUITES N11 | — | `suites_n11_graph_version_slots` BROKEN |
| R68 | the session sets search_path = ag_catalog, "$user", public (run.rs; README: AGE's documented setup) | defect | IMAGE-RESULTS §5.1, upstream 14 | 14 | `searchpath_create_graph` BROKEN, `searchpath_property_map` BROKEN, `searchpath_where_equals` BROKEN, `searchpath_unwind` BROKEN, `searchpath_edge_join` BROKEN |
| R69 | (image, no gateway rewrite) TRUNCATE fails in a database without the age extension while age is preloaded; the image installs age in every database and template1 (ROBO 11.3) | defect | IMAGE-RESULTS §5.11 ①, IMAGE-RESULTS §3.3, ROBO 11.3, upstream 13 | 13 | `image1_truncate_without_age` BROKEN |
| R70 | (rule, no rewrite) the gateway adds no column to a label table — only indexes (ddl.rs) and INHERIT (DIRECTION §4.2 rule 5); MERGE takes a 4-column vertex label for an edge label | defect | IMAGE-RESULTS §5.11 ②, upstream 15 | 15 | `image2_merge_four_column_label` BROKEN |
| R71 | (rule, no rewrite) a column added to a label table cannot be dropped again, so the gateway never adds one | defect | IMAGE-RESULTS §5.11 ③, upstream 15, apache/age #1639 | 15 | `image3_drop_added_column` BROKEN |
| R72 | results are read as agtype text (agtype.rs: ::vertex / ::edge / ::path, 1 and 1.0 kept apart), never cast to jsonb: AGE refuses a top-level scalar as json | defect | IMAGE-RESULTS §5.11 ⑤, IMAGE-RESULTS §5.4 | — | `image5_scalar_to_jsonb` BROKEN |
| R73 | (no rewrite) summary.counters come from pg_stat_xact_* (mod.rs COUNTS), because no trigger fires on a Cypher write; nothing may hang off Cypher writes | defect | IMAGE-RESULTS §5.7, upstream 17 | 17 | `image_cypher_writes_skip_triggers` BROKEN |
| R74 | MATCH (n:_Entity) reads the child tables too — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_match_parent_includes_children` HOLDS |
| R75 | label(n) of a vertex found through the parent is its own table's label — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_label_is_own_table` HOLDS |
| R76 | labels(n) has the own label only; the gateway adds the ancestors itself from pg_inherits (G3) and would list them twice if AGE started to — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_labels_lacks_ancestor` HOLDS |
| R77 | MATCH (n) sees each vertex once although a child table has two parents — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_match_all_counts_once` HOLDS |
| R78 | MERGE (n:_Entity {name: …}) finds the vertex in the child table instead of making a new one — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_merge_parent_finds_child` HOLDS |
| R79 | SET through the parent label is stored in the child table — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_set_through_parent` HOLDS |
| R80 | a relationship pattern from the parent label reaches the child's edges — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_relationship_through_parent` HOLDS |
| R81 | DETACH DELETE through the parent label deletes from the child table and the per-table statistics the counters read (mod.rs COUNTS) see it there — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_detach_delete_counts_per_table` HOLDS |
| R82 | ag_catalog.drop_label(g, '_Entity') is refused while a child inherits, so a class table cannot go by accident — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_drop_parent_label_refused` HOLDS |
| R83 | a vertex created in the child label after the INHERIT is seen through the parent (G1 makes the child table and its INHERIT before the first write) — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_create_child_after_inherit` HOLDS |
| R84 | the hierarchy is read from the catalog — ag_label joined with pg_inherits, as in bolt/README.md 'Label hierarchy' (hierarchy.rs) — and an INHERIT on a label table shows there — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_catalog_shows_hierarchy` HOLDS |
| R85 | labels(x) → (CASE label(x) WHEN 'Customer' THEN ['Customer', '_Entity'] … ELSE labels(x) END), and $class IN / a comprehension over that list — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_labels_case_with_ancestors` HOLDS |
| R86 | a node has label L when its table is L's or inherits L's, directly or not (bolt/README.md 'Label hierarchy') — label-table inheritance the gateway is to rely on (DIRECTION §4.2 rule 5, STUDIO G1/G3/G6): (:_Entity:X) stored as X, with ALTER TABLE g."X" INHERIT g."_Entity" | relied_on | STUDIO §2.5, DIRECTION §4.2 rule 5 | — | `inh_two_levels` HOLDS |
| R87 | any/all/none/single → size([x IN l WHERE p]) > 0: the comprehension may read a matched vertex | relied_on | ROBO #2, upstream 02 | 02 | `tgt_any_as_size_of_comprehension` HOLDS |
| R88 | id(x) IN list → id(coalesce(x)) IN list, with a parameter and with a literal list | relied_on | ROBO #9, upstream 09 | 09 | `tgt_id_in_list_through_coalesce` HOLDS |
| R89 | elementId(x) / toString(id(x)) → toString(id(coalesce(x))): the element id a Bolt 4.4 driver derives | relied_on | ROBO #9, upstream 09 | 09 | `tgt_tostring_id_through_coalesce` HOLDS |
| R90 | elementId(x) = e → id(x) = $__og_eid_p (a parameter that is an integer's text), or id(x) = toInteger(e) AND toString(id(coalesce(x))) = e for an expression | relied_on | STUDIO G4 | — | `tgt_element_id_compared_as_integer` HOLDS |
| R91 | elementId(x) = e → id(x) = <integer> so that AGE answers from the label table's id index (STUDIO G4: 28 ms → 2 ms at 25k vertices) | relied_on | STUDIO G4 | — | `tgt_id_equals_uses_index` HOLDS |
| R92 | a leading OPTIONAL MATCH → WITH 1 AS __og_one in front | relied_on | ROBO #5, upstream 05 | 05 | `tgt_with_before_optional_match` HOLDS |
| R93 | ORDER BY m.k of a projected map m → the key read from where the map got it (rm.name) | relied_on | ROBO #8, upstream 08 | 08 | `tgt_order_by_source_key` HOLDS |
| R94 | an aggregation next to a list comprehension → aggregates first, the comprehension in the next projection | relied_on | ROBO #3, upstream 03 | 03 | `tgt_aggregate_then_comprehension` HOLDS |
| R95 | randomUUID() → toString(pg_catalog.concat(pg_catalog.gen_random_uuid())): AGE hands a schema-qualified name to PostgreSQL's function, also inside ON CREATE SET | relied_on | ROBO #11 | — | `tgt_uuid_from_pg_function` HOLDS |
| R96 | exists(pattern) with a new variable → EXISTS { MATCH … } | relied_on | ROBO #12 | — | `tgt_exists_subquery_names_variable` HOLDS |
| R97 | n:L → label(n) = 'L'; (n:A\|B) → label(n) IN [...]; -[:A\|B]- → type(v) IN [...] | relied_on | ROBO §5.1 (surface) | — | `tgt_label_and_type_tests` HOLDS |
| R98 | a computed MERGE property → CASE WHEN e IS NULL THEN 1 / CASE WHEN e IS NULL THEN 0 ELSE 1 END ELSE e END; AGE's 'division by zero' is answered with Neo4j's null-merge error (cypher.rs guard_merge_nulls, mod.rs) | relied_on | SUITES N7 | — | `tgt_null_merge_guard` HOLDS |
| R99 | CREATE CONSTRAINT … IS UNIQUE → a unique expression index on the label's table: AGE's executor inserts through the table's indexes (ddl.rs) | relied_on | ROBO §5.3 | — | `tgt_unique_index_holds_in_cypher` HOLDS |
| R100 | summary.counters → pg_stat_get_xact_tuples_{inserted,deleted,updated} of the graph's label tables before and after the statement (mod.rs COUNTS) | relied_on | ROBO §6 | — | `tgt_counters_from_pg_stat_xact` HOLDS |
| R101 | CALL db.labels() / db.relationshipTypes() → ag_catalog.ag_label rows of the graph, kind 'v' / 'e', the two base tables left out (catalog reads: DIRECTION §4.2 rule 6) | relied_on | ROBO §5.1 (surface) | — | `tgt_catalog_lists_labels` HOLDS |
| R102 | the VLE walk (reach.rs fetch): SELECT id::text, start_id::text, end_id::text FROM <edge tables or _ag_label_edge> WHERE start_id = ANY('{…}'::ag_catalog.graphid[]); the ids are Cypher's id() | relied_on | ROBO 11.4, ROBO #13 | — | `tgt_walk_reads_edge_tables` HOLDS |
| R103 | the VLE walk's statement: the pattern replaced by id(coalesce(t)) IN coalesce($__og_reachN[toString(id(coalesce(s)))], []) | relied_on | ROBO 11.4, ROBO #13 | — | `tgt_walk_reached_map_test` HOLDS |
| R104 | SET n:L / REMOVE n:L → the vertex's row moved by SQL: DELETE FROM g._ag_label_vertex … RETURNING properties, INSERT INTO g."L" (properties) … RETURNING id, every edge's start_id/end_id pointed at the new id (split.rs relabel; DIRECTION §4.2 rule 3: the one place that writes label tables) | relied_on | ROBO #14, ROBO §7 (R3) | — | `tgt_relabel_by_sql` HOLDS |
| R105 | the cut statements carry rows as $__og_rows and find each vertex again: UNWIND $__og_rows AS __og_row OPTIONAL MATCH (v:L) WHERE id(v) = __og_row.v (split.rs) | relied_on | ROBO §5.3, ROBO §7 (R3) | — | `tgt_rows_found_again_by_id` HOLDS |
| R106 | N1's marker: MERGE … ON CREATE SET m.__og_c = true, later SET m.__og_c = null removes the key (split.rs) | relied_on | SUITES N1 | — | `tgt_merge_marker_set_and_cleared` HOLDS |
| R107 | a variable-length relationship of up to 3 hops is sent to AGE as written: its rows are Neo4j's trails (no relationship twice), the start counted only over a closed trail | relied_on | ROBO 11.4, ROBO #13 | — | `tgt_short_vle_trails_like_neo4j` HOLDS |
| R108 | -[:A\|B*1..n]- with n ≤ 3 → -[v*1..n]- and size([x IN v WHERE NOT (type(x) IN ['A', 'B'])]) = 0 (cypher.rs lift_alternatives) | relied_on | ROBO #13 | — | `tgt_short_vle_types_tested_on_list` HOLDS |
| R109 | results are read as agtype text (agtype.rs): ::vertex {id, label, properties}, ::edge {id, label, end_id, start_id, properties}, ::path, 1 and 1.0 kept apart | relied_on | IMAGE-RESULTS §5.4, IMAGE-RESULTS §6.5 | — | `tgt_agtype_text_forms` HOLDS |
| R110 | AGE's type errors answered as Neo.ClientError.Statement.TypeError (mod.rs error_code): 22023 Invalid input parameter types for agtype_… (was ParameterMissing, read from the word parameter), 0A000 … clause can only update vertex and edges / can only delete vertices and edges (was SyntaxError), XX000 container is not an agtype array | gateway-only | SUITES B10, SUITES B12, SUITES G14 | — | `gateway_type_error_codes` SKIPPED |
| R111 | AGE's type errors answered as Neo.ClientError.Statement.TypeError (mod.rs error_code) — the gateway reads the message: 22023 starting with Invalid input parameter type, XX000 container is not an agtype array | relied_on | SUITES B10, SUITES G14 | — | `tgt_type_error_message` HOLDS |
| R112 | SET r = item.props, SET n += item.props (a value AGE refuses there: not a map literal, a function call or a variable) → WITH …, item.props AS __og_setvN SET r = __og_setvN (cypher.rs bind_map_params) | surface | ANALYZER §5.3 | — | `surface_set_map_property_access` BROKEN |
| R113 | SET r = item.props → WITH …, item.props AS __og_setvN SET r = __og_setvN (cypher.rs bind_map_params) | relied_on | ANALYZER §5.3 | — | `tgt_set_map_property_bound_by_with` HOLDS |
| R114 | n {.*, k: e, .p} → CASE WHEN n IS NULL THEN null ELSE (properties(n) - ['k', 'p']) + {k: e, p: n.p} END (cypher.rs all_properties_and) | defect, relied_on | ANALYZER §5.4 | — | `analyzer_map_projection_all_null_entry` BROKEN, `analyzer_map_projection_all_key_override` HOLDS |
| R115 | n {.*, k: e, .p} → CASE WHEN n IS NULL THEN null ELSE (properties(n) - ['k', 'p']) + {k: e, p: n.p} END (cypher.rs all_properties_and): agtype - [keys] removes keys from an object, + of two objects merges them, a map literal keeps a null entry | relied_on | ANALYZER §5.4 | — | `tgt_map_projection_minus_keys` HOLDS |
| R116 | a Bolt database name under three characters → the AGE graph _og_<name> (mod.rs age_graph_name): AGE takes _og_ab as a graph name and runs Cypher in it | relied_on | SUITES N10, SUITES A14, SUITES G27 | — | `tgt_short_database_graph_name` HOLDS |
| R117 | CALL { … UNION [ALL] … } → every branch with the rows brought in: UNWIND $__og_rows AS __og_row OPTIONAL MATCH (s:L) WHERE id(s) = __og_row.s WITH __og_row, s <branch> UNION ALL UNWIND $__og_rows … (split.rs Seg::Call, SUITES B2) | relied_on | SUITES B2, SUITES G6 | — | `tgt_rows_in_every_union_branch` HOLDS |
| R118 | CALL { … UNION … }: the rows brought in on every branch (B2, could not find rte for s); CALL { … } names checked as Neo4j checks them — Variable `s` already declared in outer scope, Variable `nope` not defined (B3); a FOREACH, CALL { … } or label change inside a FOREACH or CALL { … } body cut into steps of its own (B4) — split.rs plan_in / run_on | gateway-only | SUITES B2, SUITES B3, SUITES B4, SUITES G6, SUITES G7, SUITES G8 | — | `gateway_call_subquery_union_and_names` SKIPPED |
| R119 | SHOW INDEXES / SHOW CONSTRAINTS [YIELD … [WHERE …] [RETURN …]] → the indexes ddl.rs made on the label tables, read from pg_index (procs.rs; the rows as a parameter when more follows) | surface | SUITES B6, SUITES G10, STUDIO G5b, ANALYZER §5.2 | — | `surface_show_indexes` BROKEN |
| R120 | SHOW INDEXES / SHOW CONSTRAINTS → pg_index rows of the graph's label tables, a key column's pg_get_indexdef read back as the property key (procs.rs indexes_sql / index_key): agtype_access_operator(VARIADIC ARRAY[properties, '"k"'::agtype]); AGE's own indexes (id, start_id, end_id) are plain columns and left out | relied_on | SUITES G10, STUDIO G5b, DIRECTION §4.2 rule 6 | — | `tgt_index_catalog_read` HOLDS |
| R121 | CALL dbms.components() YIELD name, versions, edition → one row: Neo4j Kernel, ['4.4.0'], community, as HELLO's agent says (procs.rs, STUDIO G5a) | surface | STUDIO G5a | — | `surface_dbms_components` BROKEN |
| R122 | CALL db.awaitIndexes(n), db.awaitIndex(name, n) → nothing (ddl.rs makes an index within the statement; procs.rs: a VOID procedure, alone no rows, in a statement no change to its rows) | surface | SUITES B7, SUITES G11 | — | `surface_await_indexes` BROKEN |
| R123 | CALL apoc.neighbors.tohop(n, '>', d) YIELD node AS m → MATCH (n)-[*1..d]->(m) WHERE id(m) <> id(n) WITH DISTINCT …, m (procs.rs tohop; over 3 hops without one type walked by reach.rs) | surface | STUDIO G5c | — | `surface_apoc_neighbors_tohop` BROKEN |
| R124 | CALL apoc.neighbors.tohop(n, '>', d) YIELD node AS m → MATCH (n)-[*1..d]->(m) WHERE id(m) <> id(n) WITH DISTINCT …, m (procs.rs tohop): each reached node once, the start left out | relied_on | STUDIO G5c | — | `tgt_tohop_as_match_distinct` HOLDS |
| R125 | CALL apoc.meta.schema() YIELD value → per label and type: count (Cypher), property types from a sample of 100 per label (Cypher), indexed / unique from ddl.rs's indexes (procs.rs meta_schema; approximate) | surface | STUDIO G5d | — | `surface_apoc_meta_schema` BROKEN |
| R126 | apoc.meta.schema() → MATCH (n) RETURN label(n), count(*) and MATCH (n:L) WHERE label(n) = 'L' WITH n LIMIT 100 RETURN properties(n) per label (procs.rs meta_schema): the sample of a parent label's own table, the children's counted under their labels | relied_on | STUDIO G5d, STUDIO §2.5 | — | `tgt_meta_schema_samples` HOLDS |
| R127 | an unknown procedure → Neo.ClientError.Procedure.ProcedureNotFound with Neo4j's wording (was SyntaxError "CALL x() inside a query is not supported"); db.index.vector.* / db.index.fulltext.* among them, with why (no vector or full-text search through SQL); an unknown YIELD column → SyntaxError Unknown procedure output (procs.rs) | gateway-only | SUITES B12, SUITES G14, SUITES A15 | — | `gateway_unknown_procedure` SKIPPED |

### Procedures and SHOW

`src/age/procs.rs`. What a client asks of a procedure here is the graph's
schema, which AGE keeps in `ag_label` and the gateway's schema commands keep
in PostgreSQL's catalog (the indexes `ddl.rs` makes on the label tables) —
catalog reads, DIRECTION §4.2 rule 6. A statement that is only the call, or
the call and a `RETURN` of what it yields, is answered from those rows; in
any other statement the call becomes `UNWIND $__og_procN AS label` (one
column yielded) or `UNWIND $__og_procN AS __og_procrowN WITH <what is in
scope>, __og_procrowN.name AS name, …`, with the rows read into
`$__og_procN` first, and everything after it is Cypher as written. A
statement that ends with the call (or with `SHOW … YIELD … WHERE …`) returns
what it yields. A `YIELD` of a column the procedure does not have is
`SyntaxError` "Unknown procedure output".

| Procedure | Answer |
|---|---|
| `db.labels()`, `db.relationshipTypes()` | `ag_label`, kind `v` / `e`, AGE's two base tables left out |
| `db.propertyKeys()` | `keys()` of every node and relationship, read through Cypher (two statements; it was a SQL read of the label tables) |
| `SHOW INDEXES` | every index on a label table whose key columns are property accesses (what `ddl.rs` makes; AGE's own on `id`, `start_id`, `end_id` are plain columns and left out), from `pg_index` / `pg_get_indexdef`, under the name `ddl.rs` gave it. Columns as Neo4j 5 (and 4.4's `uniqueness`) name them: `id` (the index's oid), `name`, `state` (`ONLINE`; `FAILED` for an index PostgreSQL holds invalid), `populationPercent` 100.0, `uniqueness`, `type` `RANGE`, `entityType`, `labelsOrTypes`, `properties`, `indexProvider` `range-1.0`, `owningConstraint` (the constraint's name for its unique index), `lastRead` / `readCount` / `trackedSince` null, `options` `{indexProvider: 'range-1.0', indexConfig: {}}`, `failureMessage`, `createStatement` |
| `SHOW CONSTRAINTS` | the unique ones: `type` `UNIQUENESS` (`RELATIONSHIP_UNIQUENESS`), `ownedIndex` = `name`, `propertyType` null, `options`, `createStatement`. A `NODE KEY` constraint reads back as `UNIQUENESS` (`ddl.rs` makes a unique index for it) |
| `dbms.components()` | one row: `Neo4j Kernel`, `['4.4.0']`, `community` — what `HELLO`'s agent says |
| `db.awaitIndexes(n)`, `db.awaitIndex(name, n)` | nothing: `ddl.rs` makes an index within the statement. Alone, no rows; in a statement, the rows go on unchanged. `awaitIndex` does not check that the index exists |
| `apoc.meta.schema()` | one row, `value`: per vertex label `{type: 'node', count, labels, properties, relationships: {}}`, per type `{type: 'relationship', count, properties}`, each property `{type, array, indexed, unique, existence: false}`. Approximate: a property's type is the one seen most in a sample of 100 elements stored under each label (`MATCH (n:L) WHERE label(n) = 'L' WITH n LIMIT 100 RETURN properties(n)`; a label's properties are its own table's and those of the tables inheriting it); dates are text here and read `STRING`; a node label's `relationships` is left empty; `labels` are the ones its table inherits. `count` is exact (`MATCH (n) RETURN label(n), count(*)`, summed down the hierarchy) |
| `apoc.neighbors.tohop(n, filter, d)` | the rewrite in the table above. The filter is APOC's (`'>'`, `'<KNOWS'`, `'KNOWS>|LIKES>'`, `''` / null for both ways), one direction for all its types; `d` from 1 to 30. Two rows the same in every variable before the call come out as one (the DISTINCT), where APOC is called for each |

There are no vector or full-text index rows: `CREATE VECTOR INDEX` /
`FULLTEXT` are accepted and not made, and nothing records them. So the
analyzer's wait for `analyzer_search_vec` to be `ONLINE`
(`embed_step.py` `_await_index_online`) sees no row and times out — the
embedding step cannot finish on this backend until vector search has a home
(it is not the gateway's: DIRECTION §4.2 rule 7). `db.index.vector.queryNodes`,
`db.index.fulltext.queryNodes` and every other procedure are
`Neo.ClientError.Procedure.ProcedureNotFound`, the message saying which ones
the backend answers.

Refused by name: such a relationship where the trails themselves matter (a
relationship or path variable on it, `count(*)` or rows per trail after it, a
lower bound above 1, two of them in one statement — AGE enumerates every
trail; `*1..30` over ten types ran the server out of memory), a choice of
labels (`:A|B`) or of relationship types in `CREATE` / `MERGE`, the label
writes *Label hierarchy* lists, and Ontological's fabric statements.

Error codes (`mod.rs` `error_code`; SUITES B10, B12): AGE's type errors are
answered as `Neo.ClientError.Statement.TypeError`, as Neo4j answers them —
22023 "Invalid input parameter types for agtype_mul" (an operator over a value
it does not take, `sum(m.name * 2)`; the gateway had read the word
"parameter" in it as `ParameterMissing`), 0A000 "… clause can only update
vertex and edges" / "can only delete vertices and edges" (a SET or DELETE of a
map or a scalar; was `SyntaxError`) and XX000 "container is not an agtype
array" (N8, `WITH {a: 1} AS n SET n.x = 1`). A missing parameter map (42P02)
stays `ParameterMissing`; a key missing from the map reads as null in AGE.

`summary.counters` come from PostgreSQL's per-transaction statistics of the
graph's label tables (`pg_stat_xact_*`), read before and after the statement:
nodes and relationships created and deleted are exact; `properties-set`
counts updated elements, not properties; `labels-added` counts one label per
node created (its table's — not the ones it has through inheritance, where
Neo4j counts every label of `(:A:B:C)`). A write sent without `BEGIN` runs in
a transaction of its own for that reason.

### Label hierarchy

A vertex in AGE is a row of one label table, so it has one label. The nodes
Studio and the analyzer write have several — an umbrella and a kind,
`(:_Entity:Customer)`, `(:ANALYZER_OWNED:TABLE)` — and are found by either.
Here the kind's table **inherits** the umbrella's (PostgreSQL's `ALTER TABLE …
INHERIT`), and a node has label L when its table is L's or inherits L's,
directly or not (`src/age/hierarchy.rs`). AGE's label scan takes the
inheriting tables along, so `MATCH (n:_Entity)` finds the Customers with
nothing rewritten — a single-label MATCH is sent as it is.

- **Storage.** `CREATE` / `MERGE (n:A:B:C {…})` stores the node under the last
  label, C (the specific one, as both clients write them), unless one of the
  labels given inherits all the others already, which is then used. Before the
  statement, in its transaction, the gateway makes the label tables that are
  missing (`SELECT ag_catalog.create_vlabel('<graph>'::cstring,
  '<label>'::cstring) WHERE NOT EXISTS (… ag_label …)`) and gives C's table
  what it does not inherit yet (`ALTER TABLE "<graph>"."C" INHERIT
  "<graph>"."B"`, the nearest label first, so what B inherits is not added
  again). Only C's table changes: B does not become an A. The data itself is
  written by AGE's Cypher, `CREATE (n:C {…})`.
- **Refused** (`Neo.ClientError.Statement.SemanticError`): C exists and holds
  nodes — in its table or one inheriting it, asked as `MATCH (n:C) RETURN
  id(n) LIMIT 1` before anything is made — and does not inherit A yet:
  inheriting would give every existing C node label A unasked ("… C already
  has nodes, and they would all become A as well. Use a label that has no
  nodes yet, or delete the C nodes first"); a cycle (C would have to inherit A,
  whose table inherits C's); a label that is a relationship type in the graph.
- **Reads** with more than one label, `n:L` and `labels()` are printed from
  the hierarchy (rows above). It is read from the catalog — `ag_label` joined
  with `pg_inherits` — once for each statement that needs it (a node pattern
  with more than one label, `n:L`, `labels()`, a label SET / REMOVE), and kept
  for nothing else; a plain `MATCH (n:L)` does not read it:

  ```sql
  SELECT l.name::text, l.kind::text, p.name::text
  FROM ag_catalog.ag_label l
  JOIN ag_catalog.ag_graph g ON g.graphid = l.graph
  LEFT JOIN pg_catalog.pg_inherits i ON i.inhrelid = l.relation::oid
  LEFT JOIN ag_catalog.ag_label p ON p.relation::oid = i.inhparent AND p.graph = l.graph
    AND p.kind = 'v' AND p.name <> '_ag_label_vertex'
  WHERE g.name = '<graph>' AND l.name NOT IN ('_ag_label_vertex', '_ag_label_edge')
  ORDER BY 1, i.inhseqno
  ```
- **Label changes** (`split.rs`). `SET n:L` of a label the node has through its
  table does nothing (no `labels-added`); `REMOVE n:L` of such a label, while
  the table's own label stays, is refused ("`n` is stored as Customer, and
  Customer's table inherits _Entity — every Customer node is _Entity …").
  A vertex moved to another table — `REMOVE n:Customer SET n:Client`, its row
  moved as before, the one place the gateway writes a label table — has the
  new table inherit the old one's parents first (DDL only; refused as above
  when the new table exists and holds nodes). Removing a node's own label
  moves it to its parent's table; setting a label below its own moves it down.
- **The id changes** when a vertex changes table: AGE's graphid carries the
  label's number. An element id a client kept from before the change (a screen
  showing the node) no longer finds it. That is AGE's limit; only a changed AGE
  would keep it.
- **Uniqueness is per table.** `CREATE CONSTRAINT … FOR (n:ANALYZER_OWNED)
  REQUIRE n.id IS UNIQUE` makes a unique index on `ANALYZER_OWNED`'s table and
  answers as before (`constraints-added: 1`), but PostgreSQL indexes are not
  inherited: it does not cover the nodes stored in `TABLE`, `COLUMN`, … whose
  tables inherit it, and two of those may share an id. The gateway does not
  emulate a constraint across tables. A constraint on the label a node is
  stored under (`FOR (n:TABLE)`) holds; the analyzer's `MERGE` on the specific
  label is what keeps one node per key.
- **Bolt nodes.** A node returned as a value (`RETURN n`) carries its table's
  one label in `Node.labels`; `labels(n)` gives the inherited ones as well.
- That AGE's scans, `MERGE`, `DELETE`, `label()` and `drop_label` follow
  PostgreSQL inheritance is AGE 1.8.0's observed behaviour (STUDIO-ON-AGE
  §2.5), not a documented one. The probe tests (`tests/age-probes/`) are to
  check it on every AGE version.

## Time limits and cancellation

A statement is cancelled — in PostgreSQL and, on virtual nodes, at the source —
when any of these happens:

| Event | Bolt answer |
|---|---|
| it runs past its limit: the driver's transaction timeout (`tx_timeout`, ms) if it sent one, otherwise the session's (`OG_BOLT_STATEMENT_TIMEOUT`, otherwise the engine's two minutes) | `FAILURE Neo.ClientError.Transaction.TransactionTimedOut`, with what to change; the session goes on after `RESET`, as after any failure |
| the client closes the connection, or it breaks | nobody to answer — the query stops instead of running on |
| the client sends `RESET` while it runs | `FAILURE Neo.TransientError.Transaction.Terminated`, then `IGNORED`, `SUCCESS` |
| the backend is cancelled from outside (`pg_cancel_backend`) | `FAILURE Neo.TransientError.Transaction.Terminated` |
| the gateway itself dies | PostgreSQL notices within about two seconds (`client_connection_check_interval`, not available on Windows) |

The limit is the engine's (`og.statement_timeout`, see `docs/cypher.md`): the
gateway only passes the driver's value on. While a query runs a second thread
reads the client socket — that is the only way to see a close behind the PULL a
driver has already pipelined — and sends PostgreSQL a cancel request when the
client is gone or sends `RESET`. It also cancels a few seconds after the limit
if the engine has not, as a backstop.

`start.sh` starts it alongside PostgreSQL. Nothing on the PostgreSQL path
depends on it: stop the gateway and every `og_cypher()` caller is unaffected.

## How the two worlds line up

| Neo4j | Here |
|---|---|
| database (`session(database="x")`) | **graph** `x` |
| user / password in `HELLO` | **PostgreSQL role** and its password — there is no second user store |
| `Node` | `{_id, _type, …}` from `og_cypher()`, re-encoded as a Bolt `Node` |
| `Relationship` | `{_id, _type, _src, _dst, …}`, re-encoded as a Bolt `Relationship` |
| explicit transaction | PostgreSQL transaction on the session's connection |
| `Neo4jError` | the compiler's message, verbatim, under a `Neo.ClientError.*` code |

Permissions, RLS and audit stay PostgreSQL's. A role that cannot see a row over
psql cannot see it over Bolt — the gateway never connects as anyone but the
authenticated user.

## Support matrix

Stated precisely, because "supported" without a matrix is not a claim
(constitution, technology constraints).

| | State |
|---|---|
| **Bolt 4.4** | supported — the version every current driver can negotiate |
| Bolt 3.x, 5.x | **not spoken**. This is not the limit it sounds like: a driver proposes a *range*, so the current Python driver (6.2) settles on 4.4 and connects. Negotiation fails cleanly for anything that cannot |
| Messages | `HELLO` `RUN` `PULL` `DISCARD` `BEGIN` `COMMIT` `ROLLBACK` `RESET` `GOODBYE` `ROUTE` |
| PackStream | null, bool, int (all widths), float, string, list, dictionary, structure |
| Graph types | `Node`, `Relationship` |
| `Path` | not encoded as a Path structure; a path variable arrives as a list of hops |
| Temporal / spatial types | **not supported** — a parameter carrying one is rejected, not silently mangled |
| Routing (`neo4j://`) | answers with this single server; a real routing table belongs to spec 007 |
| TLS | not terminated here; put a TLS proxy in front |
| `EXPLAIN` / `PROFILE` | accepted: the query is classified, not run, and the summary's `type` is `r` or `w`. No plan — see `docs/cypher.md` |
| `summary.counters` | nodes/relationships created and deleted, properties set, labels added, indexes and constraints added and removed |
| `CALL {}`, GDS | not supported — spec 003 does not support them, and the transport does not add semantics |
| `CALL` procedures, APOC | whatever spec 003 supports; `docs/cypher.md` has the registry |

Cypher coverage is spec 003's, unchanged: `docs/cypher.md` is the authority, and
`UNION` and `shortestPath` fail here exactly as they fail over psql.

`EXPLAIN` is the one place the gateway looks at a query at all, and it still
does not parse one: it asks `og_cypher_check()` whether the statement writes, so
the read/write verdict a client acts on is the engine's, reached through a
second transport rather than reimplemented behind it.

## Driving it from an MCP server

Neo4j's published MCP server, `mcp-neo4j-cypher`, runs against this gateway
unmodified — it is only a Neo4j driver plus three tools. What that takes, and
where it still falls short, is worked end to end in
[`examples/meeting-rooms/`](../examples/meeting-rooms/), whose `verify_mcp.py`
prints the compatibility matrix as a test.

## Tests

```bash
cargo test                      # PackStream round trips, chunk boundaries, the AGE translation
python3 ../tests/common/neo4j-movies/run.py
```

The second one is the real gate: the official Neo4j Movie Graph sample, run
over Bolt with **Neo4j's own driver**, compared query by query against both the
PostgreSQL path and a live Neo4j. Testing our server with our own client would
not be evidence.
