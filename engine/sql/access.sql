-- ============================================================================
-- Ontological — public access paths (spec 001 FR-013/014, spec 002 FR-031)
--
-- Everything here is LANGUAGE SQL on purpose: PostgreSQL inlines simple
-- set-returning SQL functions into the calling query, so the planner sees the
-- adjacency scan itself — statistics, join order, parallelism and all.  A
-- PL/pgSQL or C set-returning function would be an optimisation barrier, which
-- is precisely the mistake Constitution principle II forbids.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- Neighbour expansion. One heap tuple per 256 neighbours.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_expand(src int8, etypes int4[], dir "char")
RETURNS TABLE (nbr int8, eid int8)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 50 AS $$
    SELECT u.nbr, u.eid
      FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
     WHERE a.src = og_expand.src
       AND a.dir = og_expand.dir
       AND (og_expand.etypes IS NULL OR a.etype = ANY (og_expand.etypes))
$$;

COMMENT ON FUNCTION og_expand IS
  'Expand one node''s neighbours (spec 001 FR-002). Sequential array read, not degree index probes.';

-- Batched expansion: one call for many start points, so a multi-hop plan does
-- not degenerate into a round trip per row (spec 007 FR-012 uses the same shape).
CREATE FUNCTION og_expand_batch(srcs int8[], etypes int4[], dir "char")
RETURNS TABLE (src int8, nbr int8, eid int8)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 500 AS $$
    SELECT a.src, u.nbr, u.eid
      FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
     WHERE a.src = ANY (og_expand_batch.srcs)
       AND a.dir = og_expand_batch.dir
       AND (og_expand_batch.etypes IS NULL OR a.etype = ANY (og_expand_batch.etypes))
$$;

-- ---------------------------------------------------------------------------
-- Subtype-aware scans. The label join below is THE constant-time inheritance
-- test (spec 002 FR-009/010) — note the complete absence of recursion.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_subtype_ids(root int4)
RETURNS TABLE (type_id int4)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 8 AS $$
    SELECT DISTINCT d.type_id
      FROM og_catalog.type_label a
      JOIN og_catalog.type_label d
        ON d.graph_id = a.graph_id AND d.lft >= a.lft AND d.rgt <= a.rgt
     WHERE a.type_id = og_subtype_ids.root
$$;

CREATE FUNCTION og_nodes(root int4)
RETURNS TABLE (id int8, type_id int4)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000 AS $$
    SELECT n.id, n.type_id FROM og_data.og_node n
     WHERE n.type_id IN (SELECT og_subtype_ids(og_nodes.root))
$$;

CREATE FUNCTION og_edges(root int4)
RETURNS TABLE (id int8, type_id int4, src int8, dst int8)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000 AS $$
    SELECT e.id, e.type_id, e.src, e.dst FROM og_data.og_edge e
     WHERE e.type_id IN (SELECT og_subtype_ids(og_edges.root))
$$;

-- ---------------------------------------------------------------------------
-- Name resolution
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_type_id(graph text, type_name text)
RETURNS int4
LANGUAGE sql STABLE PARALLEL SAFE AS $$
    SELECT t.type_id FROM og_catalog.type t
      JOIN og_catalog.graph g ON g.graph_id = t.graph_id
     WHERE g.name = og_type_id.graph AND t.name = og_type_id.type_name
$$;

-- ---------------------------------------------------------------------------
-- Introspection views — spec 002 FR-031, consumed by spec 008 and the portal.
-- ---------------------------------------------------------------------------
CREATE VIEW og_type_view AS
SELECT g.name  AS graph,
       t.type_id,
       t.name,
       CASE t.kind WHEN 'e' THEN 'entity' WHEN 'r' THEN 'relation' ELSE 'attribute' END AS kind,
       t.is_abstract,
       t.storage_table,
       t.iri,
       l.depth,
       l.lft,
       l.rgt,
       ARRAY(SELECT p.name FROM og_catalog.type p
              JOIN og_catalog.type_parent tp ON tp.parent_id = p.type_id
             WHERE tp.type_id = t.type_id ORDER BY p.name) AS parents
  FROM og_catalog.type t
  JOIN og_catalog.graph g ON g.graph_id = t.graph_id
  LEFT JOIN og_catalog.type_label l ON l.type_id = t.type_id AND l.path_id = 0;

CREATE VIEW og_property_view AS
SELECT g.name AS graph, t.name AS type_name, p.name AS property,
       p.data_type, p.column_name, p.required, p.is_key
  FROM og_catalog.property p
  JOIN og_catalog.type t  ON t.type_id = p.type_id
  JOIN og_catalog.graph g ON g.graph_id = t.graph_id;

CREATE VIEW og_role_view AS
SELECT g.name AS graph, t.name AS relation, r.name AS role,
       pt.name AS player_type, r.ordinal, r.card_min, r.card_max
  FROM og_catalog.role r
  JOIN og_catalog.type t   ON t.type_id = r.rel_type_id
  JOIN og_catalog.graph g  ON g.graph_id = t.graph_id
  LEFT JOIN og_catalog.type pt ON pt.type_id = r.player_type_id;

-- Relational view of the graph so BI/ETL tooling can read it as plain SQL
-- (spec 005 FR-011).
CREATE VIEW og_node_view AS
SELECT n.id, t.name AS type_name, g.name AS graph
  FROM og_data.og_node n
  JOIN og_catalog.type t  ON t.type_id = n.type_id
  JOIN og_catalog.graph g ON g.graph_id = t.graph_id;

CREATE VIEW og_edge_view AS
SELECT e.id, t.name AS type_name, g.name AS graph, e.src, e.dst
  FROM og_data.og_edge e
  JOIN og_catalog.type t  ON t.type_id = e.type_id
  JOIN og_catalog.graph g ON g.graph_id = t.graph_id;

-- ---------------------------------------------------------------------------
-- Variable-length traversal — spec 003 FR-004.
--
-- Trail semantics: an edge may not repeat within a path, which is what stops a
-- cycle from expanding forever.  Written as a SQL function so it can be
-- LATERAL-joined per start row instead of materialising the whole graph.
--
-- Note this recursion walks DATA, not the type hierarchy — the constitution
-- bans the latter (principle IV), never the former.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_vlp(src int8, etypes int4[], dir "char", minhop int, maxhop int)
RETURNS TABLE (node int8, depth int, path int8[])
LANGUAGE sql STABLE PARALLEL SAFE ROWS 100 AS $$
    WITH RECURSIVE walk(node, depth, path) AS (
        SELECT og_vlp.src, 0, ARRAY[]::int8[]
      UNION ALL
        SELECT u.nbr, w.depth + 1, w.path || u.eid
          FROM walk w
          JOIN og_data.og_adj a
            ON a.src = w.node
           AND (og_vlp.dir = 'b'::"char" OR a.dir = og_vlp.dir)
           AND (og_vlp.etypes IS NULL OR a.etype = ANY (og_vlp.etypes))
          CROSS JOIN LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
         WHERE w.depth < og_vlp.maxhop
           AND NOT (u.eid = ANY (w.path))
    )
    SELECT node, depth, path FROM walk
     WHERE depth >= og_vlp.minhop AND depth <= og_vlp.maxhop
$$;

-- Reachability, in plain SQL — the same question `og_vlp` answers when nobody
-- asked for the path.
--
-- The recursion carries no path array and the branch is `UNION`, not
-- `UNION ALL`, so PostgreSQL deduplicates the worktable against everything
-- already produced. That turns trail enumeration (degree^k rows) into a
-- level-by-level sweep whose worktable can never exceed the number of
-- reachable nodes. It is not a true visited set — a node found at depth 2 is
-- emitted again at depth 3 because `(node, depth)` is a different row — so the
-- work is O(k · |V|) rather than O(|V| + |E|). Cheap to have, and it needs no
-- Rust: see `og_reach()` for the version that does keep a visited set.
CREATE FUNCTION og_reach_sql(src int8, etypes int4[], dir "char", minhop int, maxhop int)
RETURNS TABLE (node int8, depth int)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000 AS $$
    WITH RECURSIVE walk(node, depth) AS (
        SELECT og_reach_sql.src, 0
      UNION
        SELECT u.nbr, w.depth + 1
          FROM walk w
          JOIN og_data.og_adj a
            ON a.src = w.node
           AND (og_reach_sql.dir = 'b'::"char" OR a.dir = og_reach_sql.dir)
           AND (og_reach_sql.etypes IS NULL OR a.etype = ANY (og_reach_sql.etypes))
          CROSS JOIN LATERAL unnest(a.nbr) AS u(nbr)
         WHERE w.depth < og_reach_sql.maxhop
    )
    SELECT node, min(depth)::int FROM walk
     WHERE depth >= og_reach_sql.minhop
     GROUP BY node
$$;

COMMENT ON FUNCTION og_reach_sql IS
  'Reachability without paths, in SQL. See og_reach() for the visited-set version.';

-- `og_reach` is written in Rust, and pgrx gives every set-returning function
-- PostgreSQL's default guess of 1000 rows. `og_vlp` declares 100. Two functions
-- that answer the same question must not be costed an order of magnitude apart
-- for a reason that has nothing to do with either — the planner would pick
-- different join orders for the two and the comparison would measure the guess.
ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4) ROWS 100;

-- ---------------------------------------------------------------------------
-- Row → JSON helpers used when a query returns a whole node/relationship whose
-- concrete type is not known at compile time.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_type_name(type_id int4)
RETURNS text LANGUAGE sql STABLE PARALLEL SAFE AS $$
    SELECT name FROM og_catalog.type WHERE type_id = og_type_name.type_id
$$;

CREATE FUNCTION og_node_json(id int8)
RETURNS jsonb LANGUAGE plpgsql STABLE AS $$
DECLARE
    t    record;
    raw  jsonb;
    mapped jsonb;
BEGIN
    SELECT ty.type_id, ty.name, ty.storage_table INTO t
      FROM og_data.og_node n JOIN og_catalog.type ty ON ty.type_id = n.type_id
     WHERE n.id = og_node_json.id;
    IF NOT FOUND THEN RETURN NULL; END IF;

    EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table)
      INTO raw USING og_node_json.id;
    IF raw IS NULL THEN RETURN NULL; END IF;

    -- Physical column -> declared property name.
    SELECT jsonb_object_agg(p.name, raw -> p.column_name) INTO mapped
      FROM og_catalog.property p
     WHERE p.type_id = t.type_id
       AND raw ? p.column_name
       AND jsonb_typeof(raw -> p.column_name) <> 'null';

    RETURN jsonb_build_object('_id', og_node_json.id, '_type', t.name)
           || COALESCE(mapped, '{}'::jsonb)
           || (CASE WHEN jsonb_typeof(raw -> '__ext') = 'object'
                     THEN raw -> '__ext' ELSE '{}'::jsonb END);
END $$;

CREATE FUNCTION og_edge_json(id int8)
RETURNS jsonb LANGUAGE plpgsql STABLE AS $$
DECLARE
    t    record;
    raw  jsonb;
    mapped jsonb;
BEGIN
    SELECT ty.type_id, ty.name, ty.storage_table, e.src, e.dst INTO t
      FROM og_data.og_edge e JOIN og_catalog.type ty ON ty.type_id = e.type_id
     WHERE e.id = og_edge_json.id;
    IF NOT FOUND THEN RETURN NULL; END IF;

    EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table)
      INTO raw USING og_edge_json.id;
    IF raw IS NULL THEN RETURN NULL; END IF;

    SELECT jsonb_object_agg(p.name, raw -> p.column_name) INTO mapped
      FROM og_catalog.property p
     WHERE p.type_id = t.type_id
       AND raw ? p.column_name
       AND jsonb_typeof(raw -> p.column_name) <> 'null';

    RETURN jsonb_build_object('_id', og_edge_json.id, '_type', t.name,
                              '_src', t.src, '_dst', t.dst)
           || COALESCE(mapped, '{}'::jsonb)
           || (CASE WHEN jsonb_typeof(raw -> '__ext') = 'object'
                     THEN raw -> '__ext' ELSE '{}'::jsonb END);
END $$;

-- Property lookup for a node whose concrete type is unknown at compile time.
CREATE FUNCTION og_prop(id int8, prop text)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT og_node_json(og_prop.id) ->> og_prop.prop
$$;

-- History capture trigger — spec 008 FR-018..FR-023. Attached per type only
-- when og_enable_history() is called, because retaining history costs writes.
--
-- SECURITY DEFINER: the history table is a record *about* writers, so writers
-- must not hold DML on it. A trigger function runs with its own privileges, not
-- the caller's, which is exactly the separation wanted here.
CREATE FUNCTION og_capture_history() RETURNS trigger LANGUAGE plpgsql
SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    eid int8;
    op  "char";
    doc jsonb;
BEGIN
    IF TG_OP = 'DELETE' THEN
        eid := OLD.id; op := 'd'; doc := to_jsonb(OLD);
    ELSE
        eid := NEW.id;
        op  := CASE TG_OP WHEN 'INSERT' THEN 'i'::"char" ELSE 'u'::"char" END;
        doc := to_jsonb(NEW);
    END IF;

    UPDATE og_data.og_history SET valid_to = now()
     WHERE entity_id = eid AND valid_to IS NULL;

    INSERT INTO og_data.og_history (entity_id, is_edge, op, payload)
    VALUES (eid, doc ? 'src', op, doc);

    RETURN NULL;
END $$;

-- ---------------------------------------------------------------------------
-- TypeQL ↔ storage mapping — spec 010 FR-043.
--
-- TypeQL reifies relations as nodes and materialises attributes as shared,
-- value-deduplicated instances. That is a deliberate mapping, not a hidden one:
-- these two views state it in SQL, so anyone reading the graph through SQL,
-- Cypher or PostgREST can see exactly what a `has` and a role assignment are.
-- ---------------------------------------------------------------------------

-- One row per ownership: which instance owns which attribute value.
CREATE VIEW og_typeql_attribute AS
    SELECT e.src                     AS owner_id,
           ot.name                   AS owner_type,
           at.name                   AS attribute_type,
           og_node_json(e.dst) ->> 'val' AS value,
           e.dst                     AS attribute_id
      FROM og_data.og_edge  e
      JOIN og_catalog.type  ht ON ht.type_id = e.type_id AND ht.name = '$has'
      JOIN og_data.og_node  o  ON o.id  = e.src
      JOIN og_catalog.type  ot ON ot.type_id = o.type_id
      JOIN og_data.og_node  a  ON a.id  = e.dst
      JOIN og_catalog.type  at ON at.type_id = a.type_id;

COMMENT ON VIEW og_typeql_attribute IS
  'Spec 010 FR-043: TypeQL attribute ownership. An attribute instance is shared by value, so one row here per (owner, attribute) pair.';

-- One row per role assignment of a reified relation instance.
CREATE VIEW og_typeql_role AS
    SELECT rp.edge_id     AS relation_id,
           rt.name        AS relation_type,
           r.name         AS role,
           rp.player_id   AS player_id,
           pt.name        AS player_type
      FROM og_data.og_role_player rp
      JOIN og_catalog.role  r  ON r.role_id = rp.role_id
      JOIN og_data.og_node  rn ON rn.id = rp.edge_id
      JOIN og_catalog.type  rt ON rt.type_id = rn.type_id
      JOIN og_data.og_node  p  ON p.id  = rp.player_id
      JOIN og_catalog.type  pt ON pt.type_id = p.type_id;

COMMENT ON VIEW og_typeql_role IS
  'Spec 010 FR-043: TypeQL role assignments. Relations are reified as nodes, so relation_id is a node id and a relation may carry three or more roles.';


-- ============================================================================
-- Schema evolution helpers — spec 012.
-- ============================================================================

-- Would this text cast to that type? Migration casts through text on purpose:
-- `3.5::int8` rounds, `'3.5'::int8` refuses, and a type change that silently
-- rounds is not one anybody asked for.
--
-- PostgreSQL 16 answers that question directly with pg_input_is_valid(), with
-- no subtransaction per value; before 16 the only way is to try the cast and
-- catch the error, which costs a subtransaction per row (6× slower over a
-- million rows, measured).
CREATE FUNCTION og__castable(val text, typ text) RETURNS bool
LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF val IS NULL THEN RETURN true; END IF;
    IF current_setting('server_version_num')::int >= 160000 THEN
        RETURN pg_input_is_valid(val, typ);
    END IF;
    BEGIN
        EXECUTE format('SELECT %L::%s', val, typ);
        RETURN true;
    EXCEPTION WHEN others THEN
        RETURN false;
    END;
END $$;

-- Dual write during a type change (spec 012): between expand and contract the
-- new column follows the old one on every write, including raw SQL. The value
-- goes through text, so what the new type cannot hold is refused here, now.
-- A write that leaves the old column alone — the migration's own backfill —
-- is left alone. SECURITY INVOKER: it only touches NEW.
CREATE FUNCTION og__shadow_write() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    col text := TG_ARGV[0];
    sh  text := TG_ARGV[1];
    v   jsonb := to_jsonb(NEW) -> col;
BEGIN
    IF TG_OP = 'UPDATE' AND (to_jsonb(OLD) -> col) IS NOT DISTINCT FROM v THEN
        RETURN NEW;
    END IF;
    NEW := jsonb_populate_record(NEW, jsonb_build_object(sh,
             CASE WHEN v IS NULL OR v = 'null'::jsonb THEN NULL ELSE to_jsonb(v #>> '{}') END));
    RETURN NEW;
END $$;

-- The catalog of one graph as a document: types, parents, properties, roles,
-- and which settings apply. Stored with every schema version.
CREATE FUNCTION og__schema_snapshot(gid int4) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object('types', COALESCE(jsonb_agg(jsonb_build_object(
        'name', t.name,
        'kind', t.kind::text,
        'abstract', t.is_abstract,
        'parents', (SELECT COALESCE(jsonb_agg(p.name ORDER BY p.name), '[]'::jsonb)
                      FROM og_catalog.type_parent tp
                      JOIN og_catalog.type p ON p.type_id = tp.parent_id
                     WHERE tp.type_id = t.type_id),
        'properties', (SELECT COALESCE(jsonb_object_agg(pr.name, jsonb_build_object(
                                 'type', pr.data_type, 'column', pr.column_name,
                                 'required', pr.required, 'key', pr.is_key)), '{}'::jsonb)
                         FROM og_catalog.property pr WHERE pr.type_id = t.type_id),
        'roles', (SELECT COALESCE(jsonb_object_agg(r.name, jsonb_build_object(
                            'ordinal', r.ordinal,
                            'player', (SELECT name FROM og_catalog.type WHERE type_id = r.player_type_id))), '{}'::jsonb)
                    FROM og_catalog.role r WHERE r.rel_type_id = t.type_id),
        'temporal', EXISTS (SELECT 1 FROM og_catalog.temporal x WHERE x.type_id = t.type_id),
        'stamped',  EXISTS (SELECT 1 FROM og_catalog.stamped  x WHERE x.type_id = t.type_id)
    ) ORDER BY t.type_id), '[]'::jsonb))
      FROM og_catalog.type t
     WHERE t.graph_id = gid
$$;

-- The schema as it stood at a version, or at an instant. With neither, the
-- current one. "What structure was this written under?"
CREATE FUNCTION og_schema_snapshot(graph text, version int8 DEFAULT NULL, at timestamptz DEFAULT NULL)
RETURNS jsonb LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object('version', sv.version, 'changed_at', sv.changed_at,
                              'description', sv.description)
           || COALESCE(sv.snapshot, '{}'::jsonb)
      FROM og_catalog.schema_version sv
      JOIN og_catalog.graph g ON g.graph_id = sv.graph_id
     WHERE g.name = og_schema_snapshot.graph
       AND (og_schema_snapshot.version IS NULL OR sv.version <= og_schema_snapshot.version)
       AND (og_schema_snapshot.at IS NULL OR sv.changed_at <= og_schema_snapshot.at)
     ORDER BY sv.version DESC
     LIMIT 1
$$;

-- ============================================================================
-- Point-in-time context — spec 013 FR-006..FR-010.
--
-- A query names the instant it wants with two settings, and nothing else:
--
--     SET og.valid_at  = '2024-03-01';   -- when was it true in the world?
--     SET og.system_at = '2025-01-01';   -- as we knew it then
--
-- The generated type views read these through the functions below, so the
-- compiled SQL does not change with the instant and the plan cache stays valid.
-- Unset, both mean "now": the current state.
-- ============================================================================

CREATE FUNCTION og_valid_at() RETURNS timestamptz
LANGUAGE sql STABLE PARALLEL SAFE AS $$
    SELECT COALESCE(NULLIF(current_setting('og.valid_at', true), '')::timestamptz, now())
$$;

CREATE FUNCTION og_system_at() RETURNS timestamptz
LANGUAGE sql STABLE PARALLEL SAFE AS $$
    SELECT COALESCE(NULLIF(current_setting('og.system_at', true), '')::timestamptz, now())
$$;

CREATE FUNCTION og_asof_active() RETURNS bool
LANGUAGE sql STABLE PARALLEL SAFE AS $$
    SELECT COALESCE(current_setting('og.valid_at', true), '') <> ''
        OR COALESCE(current_setting('og.system_at', true), '') <> ''
$$;

-- Placed in a query that can only see the current state (adjacency-only
-- traversal, untyped scans, TypeQL). Answering a point-in-time question with
-- today's data would be a wrong answer that looks right, so it is an error.
CREATE FUNCTION og_require_current(what text) RETURNS bool
LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF og_asof_active() THEN
        RAISE EXCEPTION 'OG_TEMPORAL_UNSUPPORTED: % cannot answer a point-in-time query (og.valid_at / og.system_at is set)', what
          USING ERRCODE = '0A000',
                HINT = 'Use typed patterns and single-hop relationships over temporal types, or RESET og.valid_at and og.system_at.';
    END IF;
    RETURN true;
END $$;

-- ============================================================================
-- Write-time security — spec 014.
--
-- Fine-grained write privileges are PostgreSQL table and column grants on the
-- per-type storage tables (og_grant_write). The shared registries — og_node,
-- og_edge, og_adj, og_role_player — are written on every create and delete, so
-- a table grant on them would be a grant on every type at once. Row-level
-- security closes that: a registry row may be written only by a role that could
-- write the storage table of the type the row belongs to.
--
-- The owner and superusers bypass these policies, so the benchmark and admin
-- paths pay nothing; readers see a `true` policy the planner discards.
-- ============================================================================

CREATE FUNCTION og_type_priv(type_id int4, priv text) RETURNS bool
LANGUAGE sql STABLE AS $$
    SELECT COALESCE((
        SELECT CASE
                 -- The storage table is already gone (og_drop_type removes it
                 -- before its registry rows). Whoever may delete from the
                 -- registry at all may clean those up.
                 WHEN to_regclass(t.storage_table) IS NULL THEN true
                 WHEN og_type_priv.priv = 'DELETE'
                   THEN has_table_privilege(t.storage_table, 'DELETE')
                 ELSE has_any_column_privilege(t.storage_table, og_type_priv.priv)
               END
          FROM og_catalog.type t
         WHERE t.type_id = og_type_priv.type_id AND t.storage_table IS NOT NULL
    ), false)
$$;

ALTER TABLE og_data.og_node ENABLE ROW LEVEL SECURITY;
CREATE POLICY og_read   ON og_data.og_node FOR SELECT USING (true);
CREATE POLICY og_insert ON og_data.og_node FOR INSERT WITH CHECK (og_type_priv(type_id, 'INSERT'));
CREATE POLICY og_update ON og_data.og_node FOR UPDATE USING (og_type_priv(type_id, 'UPDATE'));
CREATE POLICY og_delete ON og_data.og_node FOR DELETE USING (og_type_priv(type_id, 'DELETE'));

ALTER TABLE og_data.og_edge ENABLE ROW LEVEL SECURITY;
CREATE POLICY og_read   ON og_data.og_edge FOR SELECT USING (true);
CREATE POLICY og_insert ON og_data.og_edge FOR INSERT WITH CHECK (og_type_priv(type_id, 'INSERT'));
CREATE POLICY og_update ON og_data.og_edge FOR UPDATE USING (og_type_priv(type_id, 'UPDATE'));
CREATE POLICY og_delete ON og_data.og_edge FOR DELETE USING (og_type_priv(type_id, 'DELETE'));

-- Adjacency segments change when an edge of their type is created or deleted.
ALTER TABLE og_data.og_adj ENABLE ROW LEVEL SECURITY;
CREATE POLICY og_read   ON og_data.og_adj FOR SELECT USING (true);
CREATE POLICY og_insert ON og_data.og_adj FOR INSERT WITH CHECK (og_type_priv(etype, 'INSERT'));
CREATE POLICY og_update ON og_data.og_adj FOR UPDATE
    USING (og_type_priv(etype, 'INSERT') OR og_type_priv(etype, 'DELETE'));
CREATE POLICY og_delete ON og_data.og_adj FOR DELETE
    USING (og_type_priv(etype, 'INSERT') OR og_type_priv(etype, 'DELETE'));

-- A role player belongs to the relation instance whose id carries its type.
ALTER TABLE og_data.og_role_player ENABLE ROW LEVEL SECURITY;
CREATE POLICY og_read   ON og_data.og_role_player FOR SELECT USING (true);
CREATE POLICY og_insert ON og_data.og_role_player FOR INSERT
    WITH CHECK (og_type_priv(og_id_type(edge_id), 'INSERT'));
CREATE POLICY og_update ON og_data.og_role_player FOR UPDATE
    USING (og_type_priv(og_id_type(edge_id), 'UPDATE'));
CREATE POLICY og_delete ON og_data.og_role_player FOR DELETE
    USING (og_type_priv(og_id_type(edge_id), 'DELETE'));

-- Did anything a reader can see change? System columns (`__x`, except the
-- `__ext` payload) and schema-migration shadow columns (`p_x__s<change>`) do
-- not count. A write that changes nothing else — `SET n.x = n.x`, or a
-- migration backfilling a shadow column — leaves no stamp and no history.
CREATE FUNCTION og__visible(doc jsonb) RETURNS jsonb
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT COALESCE(jsonb_object_agg(k, v), '{}'::jsonb)
      FROM jsonb_each(doc) AS e(k, v)
     WHERE NOT (k LIKE '\_\_%' AND k <> '__ext')
       AND k !~ '__s[0-9]+$'
$$;

-- System-managed stamps — spec 014 FR-007..FR-009.
--
-- SECURITY INVOKER on purpose: `current_user` has to be the role that ran the
-- statement. The values come from the session, never from the row, so a raw
-- `INSERT … (__created_by) VALUES ('someone else')` is overwritten here.
CREATE FUNCTION og__stamp() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND og__visible(to_jsonb(OLD)) = og__visible(to_jsonb(NEW)) THEN
        NEW.__created_at := OLD.__created_at; NEW.__created_by := OLD.__created_by;
        NEW.__updated_at := OLD.__updated_at; NEW.__updated_by := OLD.__updated_by;
        NEW.__actor := OLD.__actor;           NEW.__source := OLD.__source;
        RETURN NEW;
    END IF;
    IF TG_OP = 'INSERT' THEN
        NEW.__created_at := now();
        NEW.__created_by := current_user::text;
    ELSE
        NEW.__created_at := OLD.__created_at;
        NEW.__created_by := OLD.__created_by;
    END IF;
    NEW.__updated_at := now();
    NEW.__updated_by := current_user::text;
    NEW.__actor  := NULLIF(current_setting('og.actor', true), '');
    NEW.__source := NULLIF(current_setting('og.source', true), '');
    RETURN NEW;
END $$;

-- Bitemporal capture — spec 013 FR-001..FR-005.
--
-- Every row of a temporal type carries `__valid` (when it is true in the world)
-- and `__sys_from` (since when we have believed it). Changing or deleting a row
-- does two things to its history table `h_<type_id>`:
--
--   1. the old belief is closed:     __sys = [old.__sys_from, now)
--   2. the part of it that stays true is re-asserted, open-ended in system time:
--                                    __valid = [old.lower, X), __sys = [now, ∞)
--
-- X is `og.valid_from` when set, else now(). Set it to '-infinity' for a
-- correction ("it was never so"), or to a past or future instant for a change
-- that took effect then. SECURITY DEFINER because writers hold no privilege on
-- history tables; the stamp columns carry who did it.
CREATE FUNCTION og__temporal() RETURNS trigger LANGUAGE plpgsql
SECURITY DEFINER SET search_path = @extschema@, pg_catalog, pg_temp AS $$
DECLARE
    h    text := 'h_' || substr(TG_TABLE_NAME, 3);
    x    timestamptz := COALESCE(NULLIF(current_setting('og.valid_from', true), '')::timestamptz, now());
    vto  timestamptz := NULLIF(current_setting('og.valid_to', true), '')::timestamptz;
    doc  jsonb;
    ins  text;
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.__valid := tstzrange(x, COALESCE(vto, 'infinity'::timestamptz), '[)');
        NEW.__sys_from := now();
        RETURN NEW;
    END IF;

    IF TG_OP = 'UPDATE' AND og__visible(to_jsonb(OLD)) = og__visible(to_jsonb(NEW)) THEN
        NEW.__valid := OLD.__valid;
        NEW.__sys_from := OLD.__sys_from;
        RETURN NEW;
    END IF;

    IF x > lower(OLD.__valid) AND x >= upper(OLD.__valid) THEN
        RAISE EXCEPTION 'OG_TEMPORAL_RANGE: og.valid_from % lies after the end of the current validity %',
            x, OLD.__valid USING ERRCODE = '22023';
    END IF;

    doc := to_jsonb(OLD);
    ins := format('INSERT INTO og_data.%I SELECT * FROM jsonb_populate_record(NULL::og_data.%I, $1)', h, h);

    EXECUTE ins USING doc || jsonb_build_object(
        '__sys_to', now(),
        '__op', CASE TG_OP WHEN 'DELETE' THEN 'd' ELSE 'u' END);

    IF x > lower(OLD.__valid) THEN
        EXECUTE ins USING doc || jsonb_build_object(
            '__valid', tstzrange(lower(OLD.__valid), x, '[)'),
            '__sys_from', now(),
            '__sys_to', 'infinity'::timestamptz,
            '__op', 'v');
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    NEW.__valid := tstzrange(GREATEST(x, lower(OLD.__valid)), COALESCE(vto, upper(OLD.__valid)), '[)');
    NEW.__sys_from := now();
    RETURN NEW;
END $$;

-- System columns of one element: stamps and validity, without the leading
-- underscores. Nothing here can be written through Cypher.
CREATE FUNCTION og_meta(id int8) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    t   text;
    raw jsonb;
BEGIN
    SELECT storage_table INTO t FROM og_catalog.type WHERE type_id = og_id_type(og_meta.id);
    IF t IS NULL THEN RETURN NULL; END IF;
    EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t) INTO raw USING og_meta.id;
    IF raw IS NULL THEN RETURN NULL; END IF;
    RETURN (SELECT COALESCE(jsonb_object_agg(substr(k, 3), v), '{}'::jsonb)
              FROM jsonb_each(raw) AS e(k, v)
             WHERE k LIKE '\_\_%' AND k <> '__ext');
END $$;

-- Audit writer — spec 014 FR-010.
--
-- Readers must leave an audit row and must not be able to edit one, so nobody
-- but the owner holds DML on og_audit and every query writes through this.
-- The caller passes its own current_user (a SECURITY DEFINER function cannot
-- see it); a claim the session could not SET ROLE to is replaced by the
-- session user, so the column cannot name somebody else.
CREATE FUNCTION og__audit(q text, lang text, rows_out int8, ms float8, err text,
                          claimed_user text, act text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    who text := session_user::text;
BEGIN
    IF claimed_user IS NOT NULL
       AND EXISTS (SELECT 1 FROM pg_roles WHERE rolname = claimed_user)
       AND pg_has_role(session_user, claimed_user, 'MEMBER') THEN
        who := claimed_user;
    END IF;
    INSERT INTO og_data.og_audit
        (query, lang, rows_out, duration_ms, error_code,
         db_user, actor, source, reason, request_id, schema_version, action)
    VALUES
        (q, lang, rows_out, ms, err, who,
         NULLIF(current_setting('og.actor', true), ''),
         NULLIF(current_setting('og.source', true), ''),
         NULLIF(current_setting('og.reason', true), ''),
         NULLIF(current_setting('og.request_id', true), ''),
         (SELECT last_value FROM og_catalog.schema_version_seq),
         act);
END $$;

-- ============================================================================
-- Privileges
--
-- PostgreSQL grants EXECUTE on a new function to PUBLIC unless told otherwise,
-- and nothing here ever told it otherwise. That handed every role in the
-- cluster og_set_setting (which chooses the endpoint the embedding pipeline
-- sends text to), og_enable_rls, og_map_table and og_drop_graph. Meanwhile no
-- table carried a grant at all, so the same roles could not run a query — the
-- extension was open where it should have been shut and shut where it should
-- have been open.
--
-- So: deny by default at the function boundary, and hand access back through
-- og_grant(role, level), which also records the grant so that storage tables
-- created later inherit it. A function added after this runs is denied until
-- someone puts it in a list, which is the direction this should fail in.
-- ============================================================================

DO $$
DECLARE
    f record;
    n int := 0;
BEGIN
    FOR f IN
        SELECT p.oid::regprocedure AS sig
          FROM pg_proc p
          JOIN pg_namespace ns ON ns.oid = p.pronamespace
         WHERE ns.nspname = current_schema()
           AND p.proname LIKE 'og\_%'
    LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM PUBLIC', f.sig);
        n := n + 1;
    END LOOP;
    RAISE NOTICE
        'ontological: EXECUTE revoked from PUBLIC on % functions. Grant access with og_grant(''role'', ''read''|''write''|''admin'').',
        n;
END
$$;

COMMENT ON FUNCTION og_grant(text, text) IS
  'Grant a role standing privileges on og_catalog and og_data. Levels nest: write includes read, admin includes both. The role must already exist — roles are cluster-wide and outlive DROP EXTENSION, so this does not create one. The grant is recorded in og_catalog.grantee and replayed onto storage tables created later.';

COMMENT ON FUNCTION og_revoke(text) IS
  'Take back everything og_grant handed out, and forget the record. The role itself is left alone.';
