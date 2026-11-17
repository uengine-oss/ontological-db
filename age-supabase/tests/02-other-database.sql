-- Run as supabase_admin. AGE is preloaded for the whole cluster; what happens in
-- a database that does NOT have the age extension (e.g. Compose's _supabase)?
\pset footer off
DROP DATABASE IF EXISTS agesb_other;
CREATE DATABASE agesb_other;
\c agesb_other
\echo '### ordinary work in a database without age'
CREATE SCHEMA s;
CREATE TABLE s.t (id int PRIMARY KEY, j jsonb);
INSERT INTO s.t VALUES (1, '{"a":1}'), (2, '{"b":2}');
UPDATE s.t SET j = '{}' WHERE id = 2;
DELETE FROM s.t WHERE id = 2;
CREATE INDEX ON s.t USING gin (j);
ALTER TABLE s.t ADD COLUMN c text;
COPY s.t (id, j) FROM STDIN;
3	{"c":3}
\.
SELECT count(*) FROM s.t;
VACUUM s.t;
CREATE VIEW s.v AS SELECT id FROM s.t;
DROP VIEW s.v;
\echo '### TRUNCATE (AGE 1.8.0 utility hook looks up ag_catalog here)'
TRUNCATE s.t;
\echo '### DROP / CREATE EXTENSION of something else'
CREATE EXTENSION IF NOT EXISTS pgcrypto;
DROP EXTENSION pgcrypto;
DROP TABLE s.t;
DROP SCHEMA s;
\echo '### workaround: create the age extension in this database too'
CREATE EXTENSION age;
CREATE TABLE t2 (id int);
TRUNCATE t2;
\c postgres
DROP DATABASE agesb_other;
\echo '### TRUNCATE of an ordinary table in the database that has age'
CREATE TABLE public.agesb_trunc (id int);
INSERT INTO public.agesb_trunc VALUES (1);
TRUNCATE public.agesb_trunc;
DROP TABLE public.agesb_trunc;
