-- Apache AGE as it ships (1.8.0, unpatched), for the Bolt gateway's AGE backend (OG_BOLT_BACKEND=age).
-- Run by run.py with psql against the database named by --pg-database (dropped and made again).
-- No fabric here (no datasources, no source system): run with --skip-shared, and only clients
-- without fabric statements (Robo Architect Lite).
--
-- The graphs are not made here: the gateway makes each one on first use as the role it logs in as
-- (OG_BOLT_AGE_CREATE_GRAPHS, on by default), which then owns the graph's schema and label tables —
-- AGE needs that ownership to add a label. When psql runs as another role (--bootstrap-user), :login
-- is the gateway's role and gets what it needs in this database.
CREATE EXTENSION IF NOT EXISTS age;
\if :{?login}
GRANT USAGE ON SCHEMA ag_catalog TO :"login";
GRANT SELECT ON ag_catalog.ag_graph, ag_catalog.ag_label TO :"login";
SELECT format('GRANT CONNECT, CREATE ON DATABASE %I TO %I', current_database(), :'login') \gexec
\endif
