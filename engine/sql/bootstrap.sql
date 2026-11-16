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

-- [store-v1]
CREATE SCHEMA og_catalog;
-- [store-v1]
CREATE SCHEMA og_data;
-- [store-v1]
-- One SECURITY DEFINER function per Action (spec 014), owned by the role the
-- action runs as. Kept apart so granting an action's owner CREATE here grants
-- it nothing anywhere else.
CREATE SCHEMA og_action;

-- [store-v1]
COMMENT ON SCHEMA og_catalog IS 'Ontological type system catalog (spec 002)';
-- [store-v1]
COMMENT ON SCHEMA og_data    IS 'Ontological graph storage (spec 001)';

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Graphs — a database may hold several independent graphs (FR-022 / 001)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.graph (
    graph_id    int4        PRIMARY KEY,
    name        text        NOT NULL UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now()
);
-- [store-v1]
CREATE SEQUENCE og_catalog.graph_id_seq START 1;

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.type_id_seq START 1 MAXVALUE 262143;  -- 18 bits
-- [store-v1]
CREATE INDEX type_graph_kind_idx ON og_catalog.type (graph_id, kind);
-- [store-v1]
CREATE INDEX type_iri_idx        ON og_catalog.type (iri) WHERE iri IS NOT NULL;

-- [store-v1]
-- Inheritance DAG. Multiple inheritance => several rows per type. (FR-003)
CREATE TABLE og_catalog.type_parent (
    type_id   int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    parent_id int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    PRIMARY KEY (type_id, parent_id)
);
-- [store-v1]
CREATE INDEX type_parent_parent_idx ON og_catalog.type_parent (parent_id);

-- [store-v1]
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
-- [store-v1]
-- The index that makes `MATCH (v:Vehicle)` constant-time.
CREATE INDEX type_label_range_idx ON og_catalog.type_label (graph_id, lft, rgt);
-- [store-v1]
CREATE INDEX type_label_lft_idx   ON og_catalog.type_label (graph_id, lft);

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.property_id_seq START 1;

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.role_id_seq START 1;

-- [store-v1]
-- n-ary role players beyond (src, dst)
CREATE TABLE og_data.og_role_player (
    edge_id   int8 NOT NULL,
    role_id   int4 NOT NULL,
    player_id int8 NOT NULL,
    PRIMARY KEY (edge_id, role_id, player_id)
);
-- [store-v1]
CREATE INDEX og_role_player_player_idx ON og_data.og_role_player (player_id);

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.constraint_id_seq START 1;

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.rule_id_seq START 1;

-- [store-v1]
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

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.schema_version_seq START 1;

-- [store-v1]
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

-- [store-v1]
-- Keep the arrays inline: TOASTing them would destroy the locality we just
-- bought.  4KB payload stays under the 2KB*4 toast threshold per column.
ALTER TABLE og_data.og_adj ALTER COLUMN nbr SET STORAGE MAIN;
-- [store-v1]
ALTER TABLE og_data.og_adj ALTER COLUMN eid SET STORAGE MAIN;

-- [store-v1]
COMMENT ON TABLE og_data.og_adj IS
  'CSR-style adjacency segments (spec 001 FR-002/003). One row = up to 256 neighbours of one node for one relation type and direction.';

-- [store-v1]
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
-- [store-v1]
CREATE INDEX og_node_type_idx ON og_data.og_node (type_id, id);

-- [store-v1]
CREATE TABLE og_data.og_edge (
    id      int8 PRIMARY KEY,
    type_id int4 NOT NULL,
    src     int8 NOT NULL,
    dst     int8 NOT NULL
);
-- [store-v1]
CREATE INDEX og_edge_type_idx ON og_data.og_edge (type_id, id);
-- [store-v1]
CREATE INDEX og_edge_src_idx  ON og_data.og_edge (src);
-- [store-v1]
CREATE INDEX og_edge_dst_idx  ON og_data.og_edge (dst);

-- [store-v1]
-- Per-type local id allocation (the 36-bit local space of spec 001 FR-004)
CREATE TABLE og_data.og_id_alloc (
    type_id int4 PRIMARY KEY,
    next_id int8 NOT NULL DEFAULT 1
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Graph-level settings & runtime configuration
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.setting (
    key   text PRIMARY KEY,
    value text NOT NULL
);

-- [store-v1]
-- Who has been handed standing privileges, and at which level.
--
-- Storage tables are created at runtime, one per concrete type, so a GRANT
-- issued today has to be re-applied to a table that does not exist yet.
-- ALTER DEFAULT PRIVILEGES cannot do it: it keys on the role that creates the
-- object, and these are created by whoever calls og_create_type. Remembering
-- the grant is what lets og_create_type re-issue it.
CREATE TABLE og_catalog.grantee (
    role  text PRIMARY KEY,
    level text NOT NULL CHECK (level IN ('read', 'write', 'admin'))
);
-- [store-v1]
INSERT INTO og_catalog.setting (key, value) VALUES
    ('chunk_size',        '256'),
    ('supernode_threshold','4096'),
    ('inference_max_depth','16'),
    ('schema_version',    '1');

-- [store-v1]
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
-- [store-v1]
CREATE SEQUENCE og_catalog.embedding_id_seq START 1;

-- [store-v1]
-- Tracks what an embedding was computed from, so a changed source shows up as
-- stale instead of silently serving an outdated vector (spec 004 FR-022).
CREATE TABLE og_data.og_embedding_state (
    entity_id   int8 NOT NULL,
    prop        text NOT NULL,
    source_hash text NOT NULL,
    embedded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entity_id, prop)
);

-- [store-v1]
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

-- [store-v1]
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
-- [store-v1]
CREATE INDEX og_history_entity_idx ON og_data.og_history (entity_id, recorded_at DESC);
-- [store-v1]
CREATE INDEX og_history_valid_idx  ON og_data.og_history (valid_from, valid_to);

-- [store-v1]
CREATE TABLE og_data.og_source (
    entity_id  int8 PRIMARY KEY,
    source     text,
    ingested_at timestamptz NOT NULL DEFAULT now(),
    confidence real,
    author     text
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Semantic-web adapter support (spec 006)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.prefix (
    prefix text PRIMARY KEY,
    iri    text NOT NULL
);

-- [store-v1]
CREATE TABLE og_data.og_iri (
    entity_id int8 NOT NULL,
    iri       text PRIMARY KEY
);
-- [store-v1]
CREATE INDEX og_iri_entity_idx ON og_data.og_iri (entity_id);

-- [store-v1]
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
-- [store-v1]
CREATE INDEX og_triple_overflow_graph_idx ON og_data.og_triple_overflow (graph_id);

-- [store-v1]
-- Relational table -> graph type mappings (spec 005 FR-006..FR-010)
CREATE TABLE og_catalog.mapping (
    type_id      int4 PRIMARY KEY REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    source_table text NOT NULL,
    id_column    text NOT NULL,
    property_map jsonb NOT NULL DEFAULT '{}'::jsonb,
    writable     bool NOT NULL DEFAULT false
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Virtual fabric (experiment): external data sources and key-join relationships.
--
-- A datasource is a foreign server. Its credentials live in a user mapping,
-- which only a superuser can read; this table records what is safe to show.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.datasource (
    name        text PRIMARY KEY,
    engine      text NOT NULL,          -- 'postgres' | 'mysql' | 'mindsdb'
    server_name text NOT NULL,
    options     jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- [store-v1]
-- The fabric reads sources; it does not write to them. A foreign table would
-- pass an INSERT, UPDATE, DELETE or TRUNCATE on to its source if the source
-- account allowed it, so every table a datasource brings in carries a trigger
-- that refuses first. This holds for plain SQL as well as for Cypher.
CREATE FUNCTION og_catalog.og_refuse_source_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'datasource table %.% is read-only: the fabric does not write to source systems',
        TG_TABLE_SCHEMA, TG_TABLE_NAME
        USING HINT = 'Write through the source system itself, or MATERIALIZE the node type and write to the copy.';
END $$;

-- [store-v1]
-- A virtual relationship is a join predicate, not stored adjacency:
-- (a:src_type)-[:R]->(b:dst_type) holds when a.src_prop = b.dst_prop.
CREATE TABLE og_catalog.virtual_rel (
    type_id   int4 PRIMARY KEY REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    src_type  int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    src_prop  text NOT NULL,
    dst_type  int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    dst_prop  text NOT NULL
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Agent roles & resource limits (spec 008 FR-024..FR-029)
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.agent_role (
    name   text PRIMARY KEY,
    limits jsonb NOT NULL DEFAULT '{}'::jsonb
);

-- [store-v1]
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
-- [store-v1]
CREATE INDEX og_audit_at_idx ON og_data.og_audit (at DESC);

-- [store-v1]
-- Who wrote the row, as the database knows it and as the application claims it
-- (spec 014 FR-010). `principal` above is `session_user`, which is the pooler's
-- login under PostgREST or the Studio, not the role the statement ran as.
ALTER TABLE og_data.og_audit
    ADD COLUMN db_user        text,   -- current_user of the caller
    ADD COLUMN actor          text,   -- og.actor, application-asserted
    ADD COLUMN source         text,   -- og.source
    ADD COLUMN reason         text,   -- og.reason — why, in the caller's words
    ADD COLUMN request_id     text,   -- og.request_id
    ADD COLUMN schema_version int8,
    ADD COLUMN action         text;   -- the Action a write came through, if any

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Fine-grained write grants — spec 014 FR-001..FR-006.
--
-- The intent behind og_grant_write(), kept so that storage created later (a new
-- subtype, a new property column) receives the same privileges. The privileges
-- themselves are ordinary PostgreSQL table and column grants: that is what
-- makes them hold for raw SQL as well as for Cypher.
--   op    : 'create' | 'set' | 'delete'
--   props : NULL = every property, including undeclared ones in __ext
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.write_grant (
    role     text NOT NULL,
    type_id  int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    op       text NOT NULL CHECK (op IN ('create', 'set', 'delete')),
    props    text[],
    PRIMARY KEY (role, type_id, op)
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Actions — spec 014 FR-011..FR-016.
--
-- A named, parameterised write. Each one is backed by its own SECURITY DEFINER
-- function owned by `run_as`, so "may this caller run this action?" is answered
-- by PostgreSQL's EXECUTE check against the caller, and "what may the action
-- touch?" by the grants `run_as` holds. Neither answer depends on us.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.action (
    action_id    serial PRIMARY KEY,
    graph_id     int4 NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    name         text NOT NULL,
    lang         text NOT NULL DEFAULT 'cypher' CHECK (lang IN ('cypher', 'typeql')),
    body         text NOT NULL,
    params       jsonb NOT NULL DEFAULT '{}'::jsonb,   -- {"name": "string", ...} required keys
    precondition text,                                 -- Cypher returning one row {ok: bool}
    run_as       text NOT NULL,
    fn_name      text NOT NULL,
    description  text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (graph_id, name)
);

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Schema changes as first-class, versioned objects — spec 012.
--
-- A change moves through planned → expanded → migrating → validated →
-- contracted (or aborted). Each phase is a separate call, so a change on a large
-- graph can be expanded today, backfilled in batches, validated, and contracted
-- when the data is ready — and the record says where it stands.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.schema_change (
    change_id    serial PRIMARY KEY,
    graph_id     int4 NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    kind         text NOT NULL,
    spec         jsonb NOT NULL,
    phase        text NOT NULL DEFAULT 'planned'
                 CHECK (phase IN ('planned','expanded','migrating','validated','contracted','aborted')),
    from_version int8,
    to_version   int8,
    created_by   text NOT NULL DEFAULT current_user,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    report       jsonb NOT NULL DEFAULT '{}'::jsonb
);
-- [store-v1]
CREATE INDEX schema_change_graph_idx ON og_catalog.schema_change (graph_id, change_id DESC);

-- [store-v1]
-- What the schema looked like at each version — "what structure was this
-- written under?" (spec 012 FR-009).
ALTER TABLE og_catalog.schema_version ADD COLUMN snapshot jsonb;

-- [store-v1]
-- ---------------------------------------------------------------------------
-- Bitemporal types — spec 013.
--
-- A type listed here keeps valid time and system time on every row of its
-- storage tables, and a history table `og_data.h_<type_id>` beside each one.
-- Subtypes created later inherit the setting.
-- ---------------------------------------------------------------------------
CREATE TABLE og_catalog.temporal (
    type_id    int4 PRIMARY KEY REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    enabled_at timestamptz NOT NULL DEFAULT now()
);

-- [store-v1]
-- Types whose rows carry system-managed stamps (spec 014 FR-007..FR-009).
CREATE TABLE og_catalog.stamped (
    type_id    int4 PRIMARY KEY REFERENCES og_catalog.type(type_id) ON DELETE CASCADE
);

-- [store-v1]
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
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_parent', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_label', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.property', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.role', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.og_constraint', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.rule', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_version', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.embedding', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.compat_index', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.prefix', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.mapping', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.datasource', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.virtual_rel', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.agent_role', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.typeql_function', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.write_grant', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.action', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_change', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.temporal', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.stamped', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.action_action_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_change_change_id_seq', '');

-- [store-v1]
-- The extension script seeds these keys, so restoring them again would collide.
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                       ''inference_max_depth'', ''schema_version'')');

-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_node', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_edge', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_adj', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_id_alloc', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_role_player', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_history', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_source', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.grantee', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_audit', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_embedding_state', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_iri', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_triple_overflow', '');

-- [store-v1]
-- Sequences carry the allocation watermarks; losing them would hand out
-- identifiers that already exist.
SELECT pg_catalog.pg_extension_config_dump('og_catalog.graph_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.type_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.property_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.role_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.constraint_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.rule_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.schema_version_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_catalog.embedding_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_history_hist_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_audit_audit_id_seq', '');
-- [store-v1]
SELECT pg_catalog.pg_extension_config_dump('og_data.og_triple_overflow_id_seq', '');
