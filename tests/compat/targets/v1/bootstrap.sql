-- v1 (Ontological as Studio ships it): one database, the extension, one graph per client.
-- Run by run.py with psql against the database named by --pg-database (dropped and made again).
-- :graphs is the comma-separated list of graphs the corpus uses.
CREATE EXTENSION IF NOT EXISTS ontological CASCADE;
CREATE EXTENSION IF NOT EXISTS postgres_fdw;

-- what Studio's installer does for its graph (src/main/postgres.js bootstrap): the abstract _Entity root
SELECT og_create_graph(g) FROM unnest(string_to_array(:'graphs', ',')) AS g;
SELECT og_create_type('ontology', '_Entity', 'entity', '{}'::text[], true)
 WHERE 'ontology' = ANY (string_to_array(:'graphs', ','));

-- the source system the fabric statements reach (a schema in the same database; postgres_fdw over TCP)
\ir ../source.sql
