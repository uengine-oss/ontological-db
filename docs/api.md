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
| `og_vlp(src, etypes, dir, minhop, maxhop) → TABLE(node, depth, path)` | variable-length walk with trail semantics; `dir` `'b'` for both |
| `og_nodes(root_type) → TABLE(id, type_id)` / `og_edges(root_type)` | subtype-aware scans |
| `og_degree(src, etype, dir)` / `og_degree_all(src, dir)` | degree, used for cost estimation |
| `og_make_id(shard, type_id, local)`, `og_id_type`, `og_id_shard`, `og_id_local` | identifier encoding |

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

Audit log: `og_data.og_audit` records every `og_cypher` call with principal,
query, row count, duration and error code.
