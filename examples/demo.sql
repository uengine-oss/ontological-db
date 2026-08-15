-- ============================================================================
-- Ontological demo dataset.
--
-- Modelled to show the three things this database has that a plain property
-- graph does not:
--   1. a real type hierarchy    (Work ⊃ Film ⊃ AnimatedFilm, Person ⊃ Director)
--   2. roles on relationships   (ACTED_IN has actor / production)
--   3. embeddings on edges      (COLLABORATED_WITH carries a context vector)
--
--   psql -f examples/demo.sql
-- ============================================================================
\set ON_ERROR_STOP on

SELECT og_create_graph('default');

-- ---------------------------------------------------------------- schema ---
-- Abstract roots: never instantiated, but queryable — MATCH (w:Work) returns
-- every Film and Series without anyone maintaining a second label.
SELECT og_create_type('default', 'Work',   'entity', ARRAY[]::text[], true);
SELECT og_create_type('default', 'Film',   'entity', ARRAY['Work']);
SELECT og_create_type('default', 'AnimatedFilm', 'entity', ARRAY['Film']);
SELECT og_create_type('default', 'Series', 'entity', ARRAY['Work']);

SELECT og_create_type('default', 'Person',   'entity');
SELECT og_create_type('default', 'Director', 'entity', ARRAY['Person']);

SELECT og_add_property('default', 'Work',   'title',   'string');
SELECT og_add_property('default', 'Work',   'year',    'int');
SELECT og_add_property('default', 'Film',   'tagline', 'string');
SELECT og_add_property('default', 'AnimatedFilm', 'technique', 'string');
SELECT og_add_property('default', 'Series', 'seasons', 'int');
SELECT og_add_property('default', 'Person', 'name',    'string', required => true);
SELECT og_add_property('default', 'Person', 'born',    'int');
SELECT og_create_index('default', 'Person', 'name');

-- Roles are what a string-typed relationship cannot express: the database
-- itself refuses an ACTED_IN whose source is not a Person.
SELECT og_create_type('default', 'ACTED_IN', 'relation');
SELECT og_add_role('default', 'ACTED_IN', 'actor',      'Person', 0);
SELECT og_add_role('default', 'ACTED_IN', 'production', 'Work',   1);
SELECT og_add_property('default', 'ACTED_IN', 'role', 'string');

SELECT og_create_type('default', 'DIRECTED', 'relation');
SELECT og_add_role('default', 'DIRECTED', 'director',   'Person', 0);
SELECT og_add_role('default', 'DIRECTED', 'production', 'Work',   1);

SELECT og_create_type('default', 'COLLABORATED_WITH', 'relation');
SELECT og_add_rule('default', 'COLLABORATED_WITH', 'symmetric');
SELECT og_add_property('default', 'COLLABORATED_WITH', 'note', 'string');
-- An embedding on a RELATIONSHIP — "find collaborations like this one".
SELECT og_add_embedding('default', 'COLLABORATED_WITH', 'context', 4, 'cosine', 'note');
SELECT og_add_embedding('default', 'Work', 'summary_vec', 4, 'cosine', 'tagline');

-- ------------------------------------------------------------------ data ---
SELECT og_cypher('default', $$CREATE (:Person {name:'Keanu Reeves', born:1964})$$);
SELECT og_cypher('default', $$CREATE (:Person {name:'Carrie-Anne Moss', born:1967})$$);
SELECT og_cypher('default', $$CREATE (:Person {name:'Laurence Fishburne', born:1961})$$);
SELECT og_cypher('default', $$CREATE (:Person {name:'Hugo Weaving', born:1960})$$);
SELECT og_cypher('default', $$CREATE (:Director {name:'Lana Wachowski', born:1965})$$);
SELECT og_cypher('default', $$CREATE (:Director {name:'Lilly Wachowski', born:1967})$$);
SELECT og_cypher('default', $$CREATE (:Director {name:'Hayao Miyazaki', born:1941})$$);
SELECT og_cypher('default', $$CREATE (:Person {name:'Rumi Hiiragi', born:1987})$$);

SELECT og_cypher('default', $$CREATE (:Film {title:'The Matrix', year:1999,
  tagline:'Welcome to the Real World', summary_vec:'[0.9,0.1,0.1,0.2]'})$$);
SELECT og_cypher('default', $$CREATE (:Film {title:'The Matrix Reloaded', year:2003,
  tagline:'Free your mind', summary_vec:'[0.85,0.15,0.1,0.2]'})$$);
SELECT og_cypher('default', $$CREATE (:AnimatedFilm {title:'Spirited Away', year:2001,
  tagline:'The tunnel led Chihiro to a mysterious town', technique:'hand-drawn',
  summary_vec:'[0.1,0.9,0.2,0.1]'})$$);
SELECT og_cypher('default', $$CREATE (:AnimatedFilm {title:'My Neighbor Totoro', year:1988,
  tagline:'A gentle spirit of the forest', technique:'hand-drawn',
  summary_vec:'[0.15,0.85,0.25,0.1]'})$$);
SELECT og_cypher('default', $$CREATE (:Series {title:'Sense8', year:2015, seasons:2})$$);

SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Keanu Reeves'}), (m:Film {title:'The Matrix'})
  CREATE (p)-[:ACTED_IN {role:'Neo'}]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Carrie-Anne Moss'}), (m:Film {title:'The Matrix'})
  CREATE (p)-[:ACTED_IN {role:'Trinity'}]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Laurence Fishburne'}), (m:Film {title:'The Matrix'})
  CREATE (p)-[:ACTED_IN {role:'Morpheus'}]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Hugo Weaving'}), (m:Film {title:'The Matrix'})
  CREATE (p)-[:ACTED_IN {role:'Agent Smith'}]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Keanu Reeves'}), (m:Film {title:'The Matrix Reloaded'})
  CREATE (p)-[:ACTED_IN {role:'Neo'}]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (p:Person {name:'Rumi Hiiragi'}), (m:AnimatedFilm {title:'Spirited Away'})
  CREATE (p)-[:ACTED_IN {role:'Chihiro'}]->(m)$$);

SELECT og_cypher('default', $$
  MATCH (d:Director {name:'Lana Wachowski'}), (m:Film {title:'The Matrix'})
  CREATE (d)-[:DIRECTED]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (d:Director {name:'Lilly Wachowski'}), (m:Film {title:'The Matrix'})
  CREATE (d)-[:DIRECTED]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (d:Director {name:'Lana Wachowski'}), (m:Series {title:'Sense8'})
  CREATE (d)-[:DIRECTED]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (d:Director {name:'Hayao Miyazaki'}), (m:AnimatedFilm {title:'Spirited Away'})
  CREATE (d)-[:DIRECTED]->(m)$$);
SELECT og_cypher('default', $$
  MATCH (d:Director {name:'Hayao Miyazaki'}), (m:AnimatedFilm {title:'My Neighbor Totoro'})
  CREATE (d)-[:DIRECTED]->(m)$$);

SELECT og_cypher('default', $$
  MATCH (a:Director {name:'Lana Wachowski'}), (b:Director {name:'Lilly Wachowski'})
  CREATE (a)-[:COLLABORATED_WITH {note:'co-directed science fiction',
                                  context:'[0.9,0.1,0.0,0.1]'}]->(b)$$);
SELECT og_cypher('default', $$
  MATCH (a:Person {name:'Keanu Reeves'}), (b:Person {name:'Carrie-Anne Moss'})
  CREATE (a)-[:COLLABORATED_WITH {note:'co-starred in an action film',
                                  context:'[0.8,0.2,0.1,0.1]'}]->(b)$$);
SELECT og_cypher('default', $$
  MATCH (a:Director {name:'Hayao Miyazaki'}), (b:Person {name:'Rumi Hiiragi'})
  CREATE (a)-[:COLLABORATED_WITH {note:'directed a young voice actor in animation',
                                  context:'[0.1,0.9,0.2,0.0]'}]->(b)$$);

-- ------------------------------------------------------------ what to try ---
\echo ''
\echo 'Demo graph "default" is ready. Things worth running in the Studio:'
\echo ''
\echo '  MATCH (w:Work) RETURN w.title, labels(w)'
\echo '      -- one label, every subtype: Film, AnimatedFilm and Series'
\echo ''
\echo '  MATCH (p:Person)-[r:ACTED_IN]->(w:Work) RETURN p, r, w'
\echo '      -- Director is a Person, so directors appear here too'
\echo ''
\echo '  MATCH (a:Person)-[:ACTED_IN]->(:Work)<-[:ACTED_IN]-(b:Person) RETURN DISTINCT a.name, b.name'
\echo ''
\echo '  SELECT id, score, entity->>''note'' FROM og_similar(''default'', <edge id>, ''context'', 3)'
\echo '      -- similarity between RELATIONSHIPS, not just nodes'
\echo ''
\echo '  :explain MATCH (p:Person)-[:ACTED_IN]->(w:Work) RETURN w.title'
\echo '      -- the SQL PostgreSQL actually plans'
\echo ''
