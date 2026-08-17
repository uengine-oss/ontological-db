-- ============================================================================
-- Ontological — bootstrap catalog & storage schema
--
-- Implements the physical structures specified by:
--   specs/001-graph-storage-engine  (og_data: adjacency segments, type tables)
--   specs/002-ontology-type-system  (og_catalog: types, labels, roles, constraints)
--
-- Constitution I: nothing here requires a patched PostgreSQL.
-- Constitution IX: every structure below is an ordinary heap relation, so it
--                  inherits MVCC / WAL / vacuum / pg_dump for free.
-- ============================================================================

CREATE SCHEMA og_catalog;
CREATE SCHEMA og_data;

COMMENT ON SCHEMA og_catalog IS 'Ontological type system catalog (spec 002)';
COMMENT ON SCHEMA og_data    IS 'Ontological graph storage (spec 001)';

-- ---------------------------------------------------------------------------
-- Graphs — a database may hold several independent graphs (FR-022 / 001)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.graph (
    graph_id    int4        PRIMARY KEY,
    name        text        NOT NULL UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE SEQUENCE og_catalog.graph_id_seq START 1;

-- ---------------------------------------------------------------------------
-- Types — entity / relation / attribute (FR-001 / 002)
--
-- type_id is embedded in every node & edge identifier, therefore it is
-- globally unique and never reused.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.type (
    type_id       int4        PRIMARY KEY,
    graph_id      int4        NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    name          text        NOT NULL,
    kind          "char"      NOT NULL,   -- 'e' entity | 'r' relation | 'a' attribute
    is_abstract   bool        NOT NULL DEFAULT false,
    storage_table text,                   -- og_data.<table>, NULL for abstract
    iri           text,                   -- RDF/OWL identity (spec 006)
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (graph_id, name),
    CHECK (kind IN ('e', 'r', 'a'))
);
CREATE SEQUENCE og_catalog.type_id_seq START 1 MAXVALUE 262143;  -- 18 bits
CREATE INDEX type_graph_kind_idx ON og_catalog.type (graph_id, kind);
CREATE INDEX type_iri_idx        ON og_catalog.type (iri) WHERE iri IS NOT NULL;

-- Inheritance DAG. Multiple inheritance => several rows per type. (FR-003)
CREATE TABLE og_catalog.type_parent (
    type_id   int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    parent_id int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    PRIMARY KEY (type_id, parent_id)
);
CREATE INDEX type_parent_parent_idx ON og_catalog.type_parent (parent_id);

-- ---------------------------------------------------------------------------
-- Interval (nested-set) labels — THE core of constant-time subtype resolution
--
--   X is a subtype of Y   <=>   EXISTS label rows lx, ly with
--                               ly.lft <= lx.lft AND lx.rgt <= ly.rgt
--
-- A single indexed range comparison. No recursion, ever. (FR-009..FR-013)
-- A type reachable through N distinct parent paths owns N label rows.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.type_label (
    type_id  int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    path_id  int4 NOT NULL,            -- 0 = primary path, 1..n = extra parents
    graph_id int4 NOT NULL,
    lft      int8 NOT NULL,
    rgt      int8 NOT NULL,
    depth    int4 NOT NULL,
    PRIMARY KEY (type_id, path_id),
    CHECK (lft < rgt)
);
-- The index that makes `MATCH (v:Vehicle)` constant-time.
CREATE INDEX type_label_range_idx ON og_catalog.type_label (graph_id, lft, rgt);
CREATE INDEX type_label_lft_idx   ON og_catalog.type_label (graph_id, lft);

-- ---------------------------------------------------------------------------
-- Properties — declared properties become REAL COLUMNS on the type table.
-- Only undeclared (ad-hoc) properties fall through to the __ext jsonb column.
-- (FR-005, FR-006 / 001)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.property (
    prop_id      int4        PRIMARY KEY,
    type_id      int4        NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    name         text        NOT NULL,
    data_type    text        NOT NULL,   -- postgres type name
    column_name  text        NOT NULL,   -- physical column
    required     bool        NOT NULL DEFAULT false,
    is_key       bool        NOT NULL DEFAULT false,
    card_min     int4        NOT NULL DEFAULT 0,
    card_max     int4,                   -- NULL = unbounded
    domain_check text,                   -- CHECK expression fragment
    iri          text,
    UNIQUE (type_id, name)
);
CREATE SEQUENCE og_catalog.property_id_seq START 1;

-- ---------------------------------------------------------------------------
-- Roles — named participant slots of a relation type (TypeDB-inspired).
-- This is what Neo4j's string-typed relationships cannot express. (FR-004..006)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.role (
    role_id        int4 PRIMARY KEY,
    rel_type_id    int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    name           text NOT NULL,
    player_type_id int4 REFERENCES og_catalog.type(type_id),
    ordinal        int4 NOT NULL,        -- 0 = src, 1 = dst, 2.. = n-ary
    card_min       int4 NOT NULL DEFAULT 0,
    card_max       int4,
    iri            text,
    -- Role specialisation: `relation authoring sub contribution,
    -- relates author as contributor` (spec 010 FR-009). Matching a supertype
    -- role has to reach every role that specialises it.
    parent_role_id int4 REFERENCES og_catalog.role(role_id),
    UNIQUE (rel_type_id, name)
);
CREATE SEQUENCE og_catalog.role_id_seq START 1;

-- n-ary role players beyond (src, dst)
CREATE TABLE og_data.og_role_player (
    edge_id   int8 NOT NULL,
    role_id   int4 NOT NULL,
    player_id int8 NOT NULL,
    PRIMARY KEY (edge_id, role_id, player_id)
);
CREATE INDEX og_role_player_player_idx ON og_data.og_role_player (player_id);

-- ---------------------------------------------------------------------------
-- Constraints not expressible as native PostgreSQL constraints (FR-015..020)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.og_constraint (
    con_id  int4 PRIMARY KEY,
    type_id int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    kind    text NOT NULL,   -- 'required'|'key'|'cardinality'|'domain'|'role_player'
    target  text,            -- property or role name
    params  jsonb NOT NULL DEFAULT '{}'::jsonb
);
CREATE SEQUENCE og_catalog.constraint_id_seq START 1;

-- ---------------------------------------------------------------------------
-- Relation characteristics driving inference (FR-027..030 / 002, spec 006 OWL)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.rule (
    rule_id        int4 PRIMARY KEY,
    rel_type_id    int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    characteristic text NOT NULL,   -- 'transitive'|'symmetric'|'reflexive'|'inverse'
    target_type_id int4 REFERENCES og_catalog.type(type_id),  -- for 'inverse'
    enabled        bool NOT NULL DEFAULT true,
    UNIQUE (rel_type_id, characteristic, target_type_id)
);
CREATE SEQUENCE og_catalog.rule_id_seq START 1;

-- ---------------------------------------------------------------------------
-- TypeQL function declarations (spec 010 FR-044).
--
-- Stored verbatim so a TypeDB schema survives a load/dump round trip even
-- where evaluation is not implemented yet. Nothing reads `body` except the
-- schema dump and the function evaluator.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.typeql_function (
    graph_id   int4 NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    name       text NOT NULL,
    signature  jsonb NOT NULL DEFAULT '{}'::jsonb,
    body       text NOT NULL,
    PRIMARY KEY (graph_id, name)
);

-- ---------------------------------------------------------------------------
-- Schema version — agents cache the schema; this is their invalidation key
-- (FR-026 / 002, FR-005 / 008)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.schema_version (
    version     int8        PRIMARY KEY,
    graph_id    int4,
    changed_at  timestamptz NOT NULL DEFAULT now(),
    description text
);
CREATE SEQUENCE og_catalog.schema_version_seq START 1;

-- ---------------------------------------------------------------------------
-- ADJACENCY SEGMENTS — the reason this database is not Apache AGE.
--
-- One row holds up to OG_CHUNK (256) neighbours of ONE node, for ONE relation
-- type, in ONE direction, as two aligned int8 arrays.  256 * 8B * 2 = 4KB,
-- which sits inside a single 8KB heap page: expanding a node is a sequential
-- read, not `degree` index probes.
--
-- Splitting on (etype, dir) gives free pruning by relation type and direction
-- (FR-003).  Splitting on seq streams supernodes without materialising them
-- (FR-014, FR-020).
-- ---------------------------------------------------------------------------
CREATE TABLE og_data.og_adj (
    src   int8   NOT NULL,
    etype int4   NOT NULL,
    dir   "char" NOT NULL,   -- 'o' outgoing | 'i' incoming
    seq   int4   NOT NULL,
    n     int4   NOT NULL,   -- live element count
    nbr   int8[] NOT NULL,
    eid   int8[] NOT NULL,
    PRIMARY KEY (src, etype, dir, seq)
) WITH (fillfactor = 80);

-- Keep the arrays inline: TOASTing them would destroy the locality we just
-- bought.  4KB payload stays under the 2KB*4 toast threshold per column.
ALTER TABLE og_data.og_adj ALTER COLUMN nbr SET STORAGE MAIN;
ALTER TABLE og_data.og_adj ALTER COLUMN eid SET STORAGE MAIN;

COMMENT ON TABLE og_data.og_adj IS
  'CSR-style adjacency segments (spec 001 FR-002/003). One row = up to 256 neighbours of one node for one relation type and direction.';

-- ---------------------------------------------------------------------------
-- Node & edge registries.
--
-- These are deliberately SKINNY: (id, type_id) only.  All properties live in
-- per-type tables as real typed columns.  The registry exists so a type-scan
-- has one anchor relation regardless of how many subtypes a hierarchy has, and
-- so role/reference validation is a single index probe.
--
-- This is NOT AGE's `_ag_label_vertex` — no property payload lives here, and
-- traversal never reads it (og_adj carries neighbour + edge ids directly).
-- ---------------------------------------------------------------------------
CREATE TABLE og_data.og_node (
    id      int8 PRIMARY KEY,
    type_id int4 NOT NULL
);
CREATE INDEX og_node_type_idx ON og_data.og_node (type_id, id);

CREATE TABLE og_data.og_edge (
    id      int8 PRIMARY KEY,
    type_id int4 NOT NULL,
    src     int8 NOT NULL,
    dst     int8 NOT NULL
);
CREATE INDEX og_edge_type_idx ON og_data.og_edge (type_id, id);
CREATE INDEX og_edge_src_idx  ON og_data.og_edge (src);
CREATE INDEX og_edge_dst_idx  ON og_data.og_edge (dst);

-- Per-type local id allocation (the 36-bit local space of spec 001 FR-004)
CREATE TABLE og_data.og_id_alloc (
    type_id int4 PRIMARY KEY,
    next_id int8 NOT NULL DEFAULT 1
);

-- ---------------------------------------------------------------------------
-- Graph-level settings & runtime configuration
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.setting (
    key   text PRIMARY KEY,
    value text NOT NULL
);
INSERT INTO og_catalog.setting (key, value) VALUES
    ('chunk_size',        '256'),
    ('supernode_threshold','4096'),
    ('inference_max_depth','16'),
    ('schema_version',    '1');

-- ---------------------------------------------------------------------------
-- Embeddings (spec 004). An embedding is a declared property with a vector
-- type; this table only records the metadata the search API needs.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.embedding (
    emb_id      int4 PRIMARY KEY,
    type_id     int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    prop        text NOT NULL,
    dims        int4 NOT NULL,
    metric      text NOT NULL DEFAULT 'cosine',
    source_prop text,
    UNIQUE (type_id, prop)
);
CREATE SEQUENCE og_catalog.embedding_id_seq START 1;

-- Tracks what an embedding was computed from, so a changed source shows up as
-- stale instead of silently serving an outdated vector (spec 004 FR-022).
CREATE TABLE og_data.og_embedding_state (
    entity_id   int8 NOT NULL,
    prop        text NOT NULL,
    source_hash text NOT NULL,
    embedded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entity_id, prop)
);

-- ---------------------------------------------------------------------------
-- Neo4j compatibility: named indexes and constraints.
--
-- Cypher written for Neo4j creates indexes by name and then *queries them by
-- that name* — `db.index.vector.queryNodes('entity_embedding_vector', …)`. The
-- underlying index here is an ordinary PostgreSQL one on a typed column, which
-- has no such name, so this table is the directory that turns a Neo4j index
-- name back into the (type, property) pair it stands for.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.compat_index (
    name        text NOT NULL,
    graph_id    int4 NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    kind        text NOT NULL,   -- 'btree'|'vector'|'fulltext'|'text'|'range'|'point'
    entity      "char" NOT NULL DEFAULT 'e',  -- 'e' node index | 'r' relationship
    type_name   text NOT NULL,
    props       text[] NOT NULL,
    options     jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (graph_id, name)
);

-- ---------------------------------------------------------------------------
-- Provenance & temporal support (spec 008)
-- ---------------------------------------------------------------------------
CREATE TABLE og_data.og_history (
    hist_id    bigserial PRIMARY KEY,
    entity_id  int8        NOT NULL,
    is_edge    bool        NOT NULL,
    op         "char"      NOT NULL,   -- 'i' insert | 'u' update | 'd' delete
    valid_from timestamptz NOT NULL DEFAULT now(),
    valid_to   timestamptz,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    txid       int8        NOT NULL DEFAULT txid_current(),
    payload    jsonb
);
CREATE INDEX og_history_entity_idx ON og_data.og_history (entity_id, recorded_at DESC);
CREATE INDEX og_history_valid_idx  ON og_data.og_history (valid_from, valid_to);

CREATE TABLE og_data.og_source (
    entity_id  int8 PRIMARY KEY,
    source     text,
    ingested_at timestamptz NOT NULL DEFAULT now(),
    confidence real,
    author     text
);

-- ---------------------------------------------------------------------------
-- Semantic-web adapter support (spec 006)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.prefix (
    prefix text PRIMARY KEY,
    iri    text NOT NULL
);

CREATE TABLE og_data.og_iri (
    entity_id int8 NOT NULL,
    iri       text PRIMARY KEY
);
CREATE INDEX og_iri_entity_idx ON og_data.og_iri (entity_id);

-- Triples that do not map losslessly onto the property-graph model are kept
-- verbatim so an export can reproduce the input (spec 006 FR-010).  This is a
-- fidelity record, NOT a second query path — nothing reads it except the
-- mapping report and the serialiser.
CREATE TABLE og_data.og_triple_overflow (
    id        bigserial PRIMARY KEY,
    graph_id  int4 NOT NULL,
    subject   text NOT NULL,
    predicate text NOT NULL,
    object    text NOT NULL,
    reason    text NOT NULL
);
CREATE INDEX og_triple_overflow_graph_idx ON og_data.og_triple_overflow (graph_id);

-- Relational table -> graph type mappings (spec 005 FR-006..FR-010)
CREATE TABLE og_catalog.mapping (
    type_id      int4 PRIMARY KEY REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    source_table text NOT NULL,
    id_column    text NOT NULL,
    property_map jsonb NOT NULL DEFAULT '{}'::jsonb,
    writable     bool NOT NULL DEFAULT false
);

-- ---------------------------------------------------------------------------
-- Agent roles & resource limits (spec 008 FR-024..FR-029)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.agent_role (
    name   text PRIMARY KEY,
    limits jsonb NOT NULL DEFAULT '{}'::jsonb
);

-- ---------------------------------------------------------------------------
-- Audit log for agent access (spec 008 FR-027)
-- ---------------------------------------------------------------------------
CREATE TABLE og_data.og_audit (
    audit_id    bigserial PRIMARY KEY,
    principal   text        NOT NULL DEFAULT session_user,
    at          timestamptz NOT NULL DEFAULT now(),
    query       text,
    lang        text,                 -- 'cypher' | 'sparql' | 'sql'
    rows_out    int8,
    duration_ms double precision,
    error_code  text
);
CREATE INDEX og_audit_at_idx ON og_data.og_audit (at DESC);

-- ---------------------------------------------------------------------------
-- Backup registration — spec 001 FR-010, spec 005 FR-024.
--
-- Tables created by a `CREATE EXTENSION` script belong to the extension, and
-- pg_dump emits only `CREATE EXTENSION` for them: their CONTENTS are skipped.
-- Every relation below holds user data, so each one has to be registered as
-- configuration data or a dump would silently restore an empty graph.
--
-- Per-type tables (og_data.n_*, og_data.e_*) are created at run time, not by
-- this script, so they are ordinary user tables and pg_dump already covers them.
-- ---------------------------------------------------------------------------
SELECT pg_catalog.pg_extension_config_dump('og_catalog.graph', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_parent', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_label', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.property', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.role', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.og_constraint', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.rule', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_version', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.embedding', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.compat_index', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.prefix', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.mapping', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.agent_role', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.typeql_function', '');

-- The extension script seeds these keys, so restoring them again would collide.
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                       ''inference_max_depth'', ''schema_version'')');

SELECT pg_catalog.pg_extension_config_dump('og_data.og_node', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_edge', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_adj', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_id_alloc', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_role_player', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_history', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_source', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_audit', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_embedding_state', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_iri', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_triple_overflow', '');

-- Sequences carry the allocation watermarks; losing them would hand out
-- identifiers that already exist.
SELECT pg_catalog.pg_extension_config_dump('og_catalog.graph_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.property_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.role_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.constraint_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.rule_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_version_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.embedding_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_history_hist_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_audit_audit_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_triple_overflow_id_seq', '');
