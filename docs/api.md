# SQL API reference

Every capability is reachable from plain SQL. The Studio and any agent
integration are conveniences over this surface, not privileged paths.

Generate this list yourself:

```sql
SELECT p.proname || '(' || pg_get_function_arguments(p.oid) || ') -> '
                 || pg_get_function_result(p.oid)
  FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
 WHERE n.nspname = 'public' AND p.proname LIKE 'og\_%'
 ORDER BY 1;
```

---

## Graphs and types — spec 002

| Function | Purpose |
|---|---|
| `og_create_graph(name) → int4` | create (or return) a graph namespace |
| `og_drop_graph(name)` | drop a graph and all of its storage |
| `og_create_type(graph, name, kind, parents text[] = '{}', is_abstract = false) → int4` | declare an `entity`, `relation` or `attribute` type; `parents` gives (multiple) inheritance |
| `og_drop_type(graph, name, cascade = false)` | refuses while instances exist unless `cascade` |
| `og_add_property(graph, type, prop, data_type, required = false, is_key = false)` | becomes a real column on the type table and every descendant |
| `og_create_index(graph, type, prop)` | B-tree on the property column, across the hierarchy |
| `og_add_role(graph, rel_type, role, player_type, ordinal, card_min = 0, card_max = null)` | named participant slot; `ordinal` 0 = source, 1 = target, 2+ = n-ary |
| `og_add_role_player(graph, rel_type, edge_id, role, player)` | attach an n-ary participant |
| `og_add_rule(graph, rel_type, characteristic, target_type = null)` | `transitive` / `symmetric` / `reflexive` / `inverse` |

Property types: `string`, `int`, `long`, `float`, `bool`, `datetime`, `date`,
`uuid`, `numeric`, `jsonb`, `text[]`, `int[]`, `vector(N)`.

### Inheritance

| Function | Purpose |
|---|---|
| `og_subtypes(type_id) → int4[]` | the type and every descendant, via one indexed range scan |
| `og_supertypes(type_id) → int4[]` | the type and every ancestor |
| `og_is_subtype(sub, sup) → bool` | single range comparison |
| `og_type_id(graph, name) → int4` / `og_type_name(type_id) → text` | name resolution |
| `og_relabel(graph_id)` | rebuild interval labels (diagnostics/repair) |

---

## Data — spec 001

| Function | Purpose |
|---|---|
| `og_create_node(graph, type, props = '{}') → int8` | returns the node id |
| `og_create_edge(graph, rel_type, src, dst, props = '{}') → int8` | validates role constraints, maintains both adjacency directions |
| `og_set_node_props(id, props)` | merge properties |
| `og_delete_node(id) → int8` / `og_delete_edge(id) → int8` | node delete detaches incident edges |
| `og_node_json(id) → jsonb` / `og_edge_json(id) → jsonb` | full entity as JSON, property names mapped back from columns |

### Traversal and identifiers

| Function | Purpose |
|---|---|
| `og_expand(src, etypes int4[], dir "char") → TABLE(nbr, eid)` | neighbours; `dir` is `'o'` or `'i'`. Inlinable, so the planner sees through it |
| `og_expand_batch(srcs int8[], etypes, dir)` | many start points in one call |
| `og_vlp(src, etypes, dir, minhop, maxhop) → TABLE(node, depth, path)` | variable-length walk with trail semantics; `dir` `'b'` for both. One row per path, so `degreeᵏ` rows |
| `og_reach(src, etypes, dir, minhop, maxhop) → TABLE(node, depth)` | the same walk as reachability: visited set, one row per node, `depth` is the first depth it was reached at. Bounded by `\|V\|+\|E\|` however deep the question goes |
| `og_reach_sql(...)` | pure-SQL reachability, `UNION` instead of a visited set. The floor `og_reach` is measured against |
| `og_csr_build(etypes, dir) → TABLE(nodes, edges, bytes, build_ms)` | compile the topology into **backend-local** memory. See the caveats below |
| `og_csr_reach(src, minhop, maxhop) → TABLE(node, depth)` / `og_csr_hops(src, dst, maxhop) → int` | reachability and bidirectional shortest-path length over the compiled graph |
| `og_csr_stats()` / `og_csr_drop()` | what this backend holds; free it |
| `og_nodes(root_type) → TABLE(id, type_id)` / `og_edges(root_type)` | subtype-aware scans |
| `og_degree(src, etype, dir)` / `og_degree_all(src, dir)` | degree, used for cost estimation |
| `og_make_id(shard, type_id, local)`, `og_id_type`, `og_id_shard`, `og_id_local` | identifier encoding |

Cypher picks between `og_vlp` and `og_reach` on its own: `og_reach` when the
query binds no path or relationship variable, its projection cannot observe how
many paths reach a node, and the depth is past where enumeration is cheaper.
Nothing routes to `og_csr_*` automatically, because a compiled graph is a frozen
snapshot that row-level security is never consulted about — that is a choice a
caller makes explicitly. `og_reach` and the `og_csr_*` functions are
`PARALLEL RESTRICTED`; a plan containing one runs in the leader.
Numbers and the full rule: [`deep-traversal.md`](deep-traversal.md).

### Operations

| Function | Purpose |
|---|---|
| `og_graph_stats(graph) → jsonb` | counts per type, adjacency packing ratio, supernode count |
| `og_degree_distribution(graph) → jsonb` | degree histogram |
| `og_reorganize(graph) → int8` | online repack of fragmented adjacency segments |
| `og_check_integrity() → TABLE(kind, entity_id, detail)` | cross-checks registry, typed tables and adjacency. Empty result = pass |

---

## Cypher — spec 003

| Function | Purpose |
|---|---|
| `og_cypher(graph, query, params = '{}') → SETOF jsonb` | execute |
| `og_cypher_json(graph, query, params) → jsonb` | one array, for PostgREST/RPC |
| `og_cypher_sql(graph, query) → text` | the compiled SQL — embed it in your own queries |
| `og_cypher_explain(graph, query, analyze = false) → jsonb` | compiled SQL plus the PostgreSQL plan |
| `og_cypher_check(query) → jsonb` | parse only; no database access |
| `og_cypher_columns(query) → text[]` | result column names in `RETURN` order — jsonb sorts keys, so the row cannot tell you. Parse only. Empty for `RETURN *` |

---

## TypeQL — spec 010

TypeDB 3.x TypeQL over the same graph, catalog and transaction. See
[`typeql.md`](typeql.md) for the supported syntax and the storage mapping.

| Function | Purpose |
|---|---|
| `og_typeql(graph, query, params = '{}') → SETOF jsonb` | execute one query |
| `og_typeql_script(graph, script) → int8` | run a whole `.tql` file; returns the number of blocks |
| `og_typeql_sql(graph, query) → text` | the compiled SQL for a read query |
| `og_typeql_check(query) → jsonb` | parse only; no database access |
| `og_typeql_schema(graph) → text` | dump the schema back out as TypeQL |

| View | Purpose |
|---|---|
| `og_typeql_attribute` | one row per ownership: owner, attribute type, value |
| `og_typeql_role` | one row per role assignment of a reified relation |

---

## Vectors — spec 004

| Function | Purpose |
|---|---|
| `og_add_embedding(graph, type, prop, dims, metric = 'cosine', source_prop = null)` | declare a `vector(N)` property **on a node or relationship type** and build the HNSW index |
| `og_vector_search(graph, type, prop, query, k = 10, filter = null) → TABLE(id, score, entity)` | top-k; `filter` is a SQL predicate on the same relation, so it pushes down |
| `og_similar(graph, id, prop, k = 10)` | "things like this one", relationships included |
| `og_hybrid_search(graph, type, prop, query, anchor = null, k = 10, vector_weight = 1, graph_weight = 1)` | RRF fusion of vector rank and graph proximity; returns both component scores |
| `og_vector_search_exact(graph, type, prop, query, k)` | brute-force ground truth for measuring recall |
| `og_stale_embeddings(graph) → TABLE(entity_id, type_name, prop)` | embeddings whose source property changed |
| `og_mark_embedded(entity_id, prop)` | record that an embedding is current |
| `og_embedding_stats(graph) → jsonb` | declared embeddings, dimensions, metrics |

Metrics: `cosine` (`<=>`), `l2` (`<->`), `ip` (`<#>`). Scores are normalised so
higher is always better.

---

## Interoperability — spec 005

| Function | Purpose |
|---|---|
| `og_enable_rls(graph, type, policy_expr)` | row-level security across the hierarchy; applies **mid-traversal** |
| `og_map_table(graph, source_table, type, id_column, property_map jsonb)` | expose an existing table as a node type without copying |
| `og_materialize_mapping(graph, type) → int8` | convert a mapping into native storage; queries do not change |
| `og_interop_report(graph) → jsonb` | mappings, secured types, available views |

Views: `og_node_view`, `og_edge_view`, `og_type_view`, `og_property_view`,
`og_role_view`.

---

## Semantic web — spec 006

| Function | Purpose |
|---|---|
| `og_load_rdf(graph, document, format = 'turtle') → jsonb` | Turtle / N-Triples; maps OWL classes to types, `rdfs:subClassOf` to inheritance, domain/range to roles |
| `og_dump_rdf(graph, format = 'turtle') → text` | serialise back, including unmapped triples |
| `og_mapping_report(graph) → jsonb` | what did not map, and why |
| `og_add_prefix(prefix, iri)` / `og_set_iri(graph, type, iri)` | namespace handling |

---

## Agents — spec 008

| Function | Purpose |
|---|---|
| `og_schema(graph, token_budget = null) → jsonb` | machine-readable schema; truncates by instance count when a budget is given and says so |
| `og_schema_for(graph, question) → jsonb` | the subset lexically relevant to one question |
| `og_explain_error(graph, query) → jsonb` | stable error code, message, spelling suggestions |
| `og_diagnose_empty(graph, query) → jsonb` | walks the pattern element by element and reports where it became empty |
| `og_estimate(graph, query) → jsonb` | dry-run: estimated rows, cost, and concrete advice |
| `og_create_role(name, limits jsonb)` / `og_apply_role(name)` | per-role `statement_timeout`, `work_mem`, read-only, row caps |
| `og_enable_history(graph, type)` | start capturing change history (off by default — it costs writes) |
| `og_history(id) → TABLE(recorded_at, op, payload)` | change log for one entity |
| `og_as_of(id, timestamptz) → jsonb` | state at a past instant; **errors** rather than returning the current value when no history is retained |
| `og_set_source(entity_id, source, confidence = null, author = null)` | provenance metadata |

Audit log: `og_data.og_audit` records every `og_cypher` and `og_typeql` call
with `principal` (`session_user`), query, row count, duration and error code —
and, since spec 014, `db_user` (the caller's `current_user`, accepted only if
`session_user` is a member of it), `actor`, `source` and `request_id` (from the
`og.*` settings, application-asserted), `schema_version` (the database-wide
schema version sequence, not the graph's) and `action` (the Action the write
came through). Rows are written only through the SECURITY DEFINER `og__audit`;
`write`-level roles hold no DML on `og_audit`, `og_history`, `og_source` or
the `h_*` history tables. Two limits: a failed query's audit row is rolled back
with the query (SEC-18), and any reader may call `og__audit` directly and so
*append* rows with caller-chosen query text — rows cannot be edited or deleted,
but their content is not proof.

For spec 008's `og_as_of`: it now returns NULL at an instant after the entity
was deleted, and breaks ties within one transaction by `hist_id`. Subtypes
created after `og_enable_history` are captured too.

---

## Schema evolution — spec 012

A schema change on a populated graph is four calls — expand, migrate, validate,
contract — and one row in `og_catalog.schema_change` that records where it
stands. Most kinds are catalog edits whose cost does not depend on the amount
of data; only `alter_property_type` (and `set_required` with a default) rewrite
rows, in batches. See [ADR-028](99_decisions/ADR-028-schema-change-at-catalog-level.md).

| Function | Purpose |
|---|---|
| `og_schema_plan(graph, change jsonb) → int4` | record a change and report its impact — counts, sample ids of rows that would violate it, lock notes. Changes nothing |
| `og_schema_expand(change_id) → jsonb` | add the new structure beside the old; new writes must satisfy the new rule from here on |
| `og_schema_migrate(change_id, batch = 10000) → jsonb` | move up to `batch` rows per table; resumable. Call until `{"done": true}`. Returns `{moved, done, migrated}` |
| `og_schema_validate(change_id) → jsonb` | `{phase, violations, samples}`; moves to `validated` only with zero violations |
| `og_schema_contract(change_id) → jsonb` | remove the old structure. Only from `validated` |
| `og_schema_abort(change_id) → jsonb` | undo an expand. Does not remove a property added by `add_property` |
| `og_schema_apply(graph, change) → jsonb` | all phases in one transaction; fails with `OG_SCHEMA_VIOLATION` (23514) and changes nothing if validation finds violations |
| `og_schema_changes(graph) → TABLE(change_id, kind, phase, spec, report, created_by, created_at, updated_at)` | every recorded change, newest first |
| `og_schema_snapshot(graph, version = null, at = null) → jsonb` | the graph's catalog — types, parents, properties, roles, temporal/stamped flags — as it stood at a version or an instant; current if neither |

Phases: `planned → expanded → migrating → validated → contracted`, or `aborted`
from any phase before `contracted`. Calling one out of order raises
`OG_SCHEMA_PHASE` (SQLSTATE 55000).

| `kind` | change JSON | what it does physically |
|---|---|---|
| `add_property` | `{type, property, data_type}` | `ADD COLUMN`, no rewrite; values already in `__ext` under that key move in. Optional only — use `set_required` afterwards |
| `drop_property` | `{type, property}` | contract: `DROP COLUMN`. History tables keep the column |
| `rename_property` | `{type, property, new_name}` | catalog only. Between expand and contract both names read and write the same column. The physical column keeps its old name |
| `alter_property_type` | `{type, property, to}` | shadow column `p_x__s<id>`; a trigger attached at expand (`og_c_shadow_<id>`, dropped at contract or abort) fills it on every write, raw SQL included, cast to the new type (a value the new type rejects fails at once); migrate casts through `text` in batches; contract swaps columns, rebuilds indexes on it, re-applies column grants. History tables follow |
| `set_required` | `{type, property, default?}` | `CHECK … NOT VALID` at expand, default filled in batches, `VALIDATE CONSTRAINT` (writes continue), then `SET NOT NULL` without a scan |
| `add_unique` | `{type, property}` | validate reports duplicates across the hierarchy; contract builds a unique index per concrete table |
| `rename_type` | `{type, new_name}` | catalog only; node ids do not change |
| `alter_role` | `{relation, role, player, card_min?, card_max?}` | validate checks existing edges and role players; contract updates the role |
| `add_parent` / `remove_parent` | `{type, parent}` | catalog + interval relabel; no row is rewritten. Inherited columns, temporal, stamping and write grants follow |

A property can be dropped, renamed or retyped only on the type that declares
it. Not supported: splitting or merging types, TypeQL `redefine`/`undefine`
through this pipeline, hierarchy-wide uniqueness. `og_create_index`, `og_map_table` and `og_materialize_mapping` now
bump the schema version, so cached Cypher plans are invalidated.

---

## Temporal — spec 013

Valid time (when it was true) and system time (since when we have believed it)
per row, for the types you choose. The query does not change; the instant is a
setting. See [ADR-027](99_decisions/ADR-027-bitemporal-current-plus-history.md).

| Function | Purpose |
|---|---|
| `og_enable_temporal(graph, type)` | version a type and its subtypes, including subtypes created later. Adds `__valid tstzrange` and `__sys_from` (existing rows: valid for all time, believed from now), creates `og_data.h_<type_id>`, attaches the capture trigger. Also turns on stamping |
| `og_temporal_types(graph) → text[]` | the temporal types of a graph |
| `og_valid_at()` / `og_system_at() → timestamptz` | the session's instant; `now()` when unset |
| `og_asof_active() → bool` | whether either instant is set |
| `og_require_current(what) → bool` | raises `OG_TEMPORAL_UNSUPPORTED` (SQLSTATE 0A000) when an instant is set. Compiled into query shapes that can only see the present |
| `og_meta(id) → jsonb` | an element's system columns, including `valid` and `sys_from` |

| Setting | Read or write | Meaning |
|---|---|---|
| `og.valid_at` | read | answer as of this valid time |
| `og.system_at` | read | answer as the database believed it at this time |
| `og.valid_from` | write | the change takes effect at this instant (default `now()`); `'-infinity'` is a correction |
| `og.valid_to` | write | end of validity for the new version (default open-ended, or the old end on update) |

```sql
SET og.valid_at  = '2021-06-01';
SET og.system_at = '2024-01-01';
SELECT og_cypher('kb', $$ MATCH (p:Person {name: "ann"}) RETURN p.salary $$);
```

The compiled SQL is the same for every instant. A temporal type reads three
branches: current rows; open history rows (`__sys_to = 'infinity'`, valid-time
slices still believed), visible whenever their validity contains the instant
and found through a partial GiST index on `__valid`; and closed history rows
(superseded beliefs), read only when an instant is set. So a query with no
instant still probes that partial index once per temporal type. Single hops
over temporal relationship types read the edge tables (the same three branches)
instead of the adjacency segments — always, not only when an instant is set. With an instant
set, these refuse rather than answer from the present: unlabelled node patterns
(in a graph with temporal node types), variable-length paths over temporal
relationship types, relationship variables without a single type, and TypeQL.
An update dated after the end of the current validity raises
`OG_TEMPORAL_RANGE` (22023). Raw SQL writes are versioned by the same trigger.

An update or delete with `og.valid_from` in the future leaves the value that
holds until then in an open history row, so queries with no instant keep
seeing it until that date. Not implemented: retention policy, editing past
valid-time slices.

Over Bolt, `tx_metadata` keys `valid_at`/`validAt`, `system_at`/`systemAt`,
`valid_from`/`validFrom` and `valid_to`/`validTo` become these settings for
that transaction (auto-commit `RUN` included).

---

## Write security — spec 014

Write privileges are PostgreSQL table and column grants on the per-type storage
tables, so they hold for raw SQL exactly as for Cypher. The shared registries
(`og_node`, `og_edge`, `og_adj`, `og_role_player`) carry row-level security
that lets a role write a registry row only if it could write that type's
storage table. Reads of the registries are not restricted. See
[ADR-026](99_decisions/ADR-026-write-privileges-as-postgres-grants.md).

| Function | Purpose |
|---|---|
| `og_grant_write(role, graph, type, ops text[], props text[] = null)` | `ops` ⊆ `create`, `set`, `delete`; `props` limits `create`/`set` to those properties, `null` means every property including undeclared ones. Reaches subtypes, subtypes created later, and columns added later. Grants read access if the role has none |
| `og_revoke_write(role, graph, type, ops = null)` | take back some or all operations |
| `og_type_priv(type_id, priv) → bool` | used by the registry policies: may the current role `INSERT`/`UPDATE`/`DELETE` that type's storage |
| `og_enable_stamping(graph, type)` | adds `__created_at`, `__created_by`, `__updated_at`, `__updated_by`, `__actor`, `__source`, set by trigger from `current_user`, `og.actor`, `og.source`. Values written into the row are overwritten, raw SQL included. Subtypes inherit it |
| `og_meta(id) → jsonb` | read the stamps (and validity, spec 013) |
| `og_define_action(graph, name, body, run_as, lang = 'cypher', params = '{}', precondition = null, description = null) → int4` | a named write. `params` is `{"name": "string", …}` with types `string`, `int`, `float`, `number`, `bool`, `list`, `map`, `any`; every declared key is required and no other key is accepted. `precondition` is Cypher returning `ok`. Creates `og_action.og_action_<id>(jsonb)`, `SECURITY DEFINER`, owned by `run_as` — the caller must be able to `SET ROLE run_as` |
| `og_grant_action(role, graph, name)` / `og_revoke_action(role, graph, name)` | `EXECUTE` on that action's function |
| `og_drop_action(graph, name)` | drop the action and its function |
| `og_run_action(graph, name, params = '{}') → SETOF jsonb` | run it. Needs `EXECUTE` on the action's function; the body runs with `run_as`'s privileges |

| Error | SQLSTATE | When |
|---|---|---|
| `OG_WRITE_DENIED: role 'r' may not set properties on 'Person' (property 'salary')` | 42501 | the engine's pre-check; PostgreSQL enforces the same rule either way |
| `OG_RESERVED_PROPERTY` | 42501 | writing `_id`, `_type`, `_src`, `_dst` or any `__…` property |
| `OG_ACTION_REJECTED` | 23514 | the action's precondition did not hold |
| parameter errors | 22023 | unknown, missing or mistyped action parameter |

Context settings: `og.actor`, `og.source`, `og.reason`, `og.request_id` reach
the audit log (`og_data.og_audit`); `og.actor` and `og.source` also reach
stamped rows. The Bolt gateway maps `tx_metadata` keys `actor`, `source`,
`reason`, `request_id`/`requestId` to transaction-local settings.

A `write`-level role without schema privileges that writes an undeclared
property now keeps it in `__ext` instead of failing on `og_add_property`.
Limits: TypeQL action bodies receive no parameters; the audit row of an action
names `run_as` as `db_user`, not the caller. The action name is cleared on
every exit, so a failed action does not leave it on later audit rows.
