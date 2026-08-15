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
CREATE FUNCTION og_capture_history() RETURNS trigger LANGUAGE plpgsql AS $$
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
