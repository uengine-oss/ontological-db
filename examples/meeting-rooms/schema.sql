-- The meeting-room ontology.
--
-- Declared, not inferred. This is the half of the example that has no Neo4j
-- equivalent: types, roles and property types exist in the catalog before any
-- data arrives, which is what lets `apoc.meta.schema()` answer exactly instead
-- of sampling. Everything *after* this file runs as ordinary Cypher over Bolt.

SELECT og_create_graph('meeting');

-- Entities ------------------------------------------------------------------
SELECT og_create_type('meeting', 'MeetingRoom', 'entity');
SELECT og_add_property('meeting', 'MeetingRoom', 'name',     'string', true,  true);
SELECT og_add_property('meeting', 'MeetingRoom', 'code',     'string', false, false);
SELECT og_add_property('meeting', 'MeetingRoom', 'location', 'string', false, false);
SELECT og_add_property('meeting', 'MeetingRoom', 'seats',    'int',    false, false);
SELECT og_add_property('meeting', 'MeetingRoom', 'descr',    'string', false, false);

SELECT og_create_type('meeting', 'Employee', 'entity');
SELECT og_add_property('meeting', 'Employee', 'name',  'string', true, true);
SELECT og_add_property('meeting', 'Employee', 'code',  'string', false, false);
SELECT og_add_property('meeting', 'Employee', 'title', 'string', false, false);
SELECT og_add_property('meeting', 'Employee', 'team',  'string', false, false);

SELECT og_create_type('meeting', 'Reservation', 'entity');
SELECT og_add_property('meeting', 'Reservation', 'code',       'string',   false, true);
SELECT og_add_property('meeting', 'Reservation', 'begin_time', 'datetime', true,  false);
SELECT og_add_property('meeting', 'Reservation', 'end_time',   'datetime', true,  false);
SELECT og_add_property('meeting', 'Reservation', 'purpose',    'string',   false, false);

-- Relations -----------------------------------------------------------------
-- Roles are what tell a generating model which way the arrow points; they are
-- the single most common bug in machine-written Cypher, and they are checked
-- here rather than hoped for.
SELECT og_create_type('meeting', 'RESERVED_BY', 'relation');
SELECT og_add_role('meeting', 'RESERVED_BY', 'reservation', 'Reservation', 0);
SELECT og_add_role('meeting', 'RESERVED_BY', 'reserver',    'Employee',    1);

SELECT og_create_type('meeting', 'FOR_ROOM', 'relation');
SELECT og_add_role('meeting', 'FOR_ROOM', 'reservation', 'Reservation', 0);
SELECT og_add_role('meeting', 'FOR_ROOM', 'room',        'MeetingRoom', 1);

-- The vector index is deliberately *not* declared here. It is created in
-- `load.py` with Neo4j's own `CREATE VECTOR INDEX` DDL, sent through the
-- unmodified Neo4j MCP server — which is the thing this example is trying to
-- demonstrate. See `README.md`.
