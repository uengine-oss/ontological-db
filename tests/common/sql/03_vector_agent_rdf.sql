-- Specs 004 / 006 / 008 regression.
\set ON_ERROR_STOP on
\pset pager off

SELECT og_create_graph('kb');
SELECT og_create_type('kb','Doc','entity');
SELECT og_add_property('kb','Doc','title','string');
SELECT og_add_property('kb','Doc','body','string');
SELECT og_add_embedding('kb','Doc','emb', 3, 'cosine', 'body');

SELECT og_create_type('kb','CITES','relation');
SELECT og_add_embedding('kb','CITES','context_emb', 3, 'cosine');

\echo '=== 004: node embeddings ==='
SELECT og_cypher('kb', $$CREATE (d:Doc {title:'batteries', body:'lithium chemistry', emb:'[1,0,0]'}) RETURN d.title$$);
SELECT og_cypher('kb', $$CREATE (d:Doc {title:'solar', body:'photovoltaics', emb:'[0,1,0]'}) RETURN d.title$$);
SELECT og_cypher('kb', $$CREATE (d:Doc {title:'anodes', body:'silicon anode', emb:'[0.9,0.1,0]'}) RETURN d.title$$);

SELECT id, round(score::numeric,4) AS score, entity->>'title' AS title
  FROM og_vector_search('kb','Doc','emb','[1,0,0]', 2);

\echo '=== 004: RELATIONSHIP embeddings (the Neo4j gap) ==='
SELECT og_cypher('kb', $$MATCH (a:Doc {title:'batteries'}), (b:Doc {title:'anodes'})
                        CREATE (a)-[:CITES {context_emb:'[1,0,0]'}]->(b) RETURN a.title$$);
SELECT og_cypher('kb', $$MATCH (a:Doc {title:'solar'}), (b:Doc {title:'anodes'})
                        CREATE (a)-[:CITES {context_emb:'[0,1,0]'}]->(b) RETURN a.title$$);
SELECT id, round(score::numeric,4) AS score, entity->>'_type' AS type
  FROM og_vector_search('kb','CITES','context_emb','[1,0,0]', 2);

\echo '=== 004: push-down — graph predicate INSIDE the vector search ==='
SELECT id, round(score::numeric,4) AS score, entity->>'title' AS title
  FROM og_vector_search('kb','Doc','emb','[1,0,0]', 2, $$v.p_title <> 'batteries'$$);

\echo '=== 004: ANN vs exact (recall check) ==='
SELECT (SELECT array_agg(id ORDER BY score DESC) FROM og_vector_search('kb','Doc','emb','[1,0,0]',3)) AS ann,
       (SELECT array_agg(id) FROM og_vector_search_exact('kb','Doc','emb','[1,0,0]',3)) AS exact;

\echo '=== 008: machine-readable schema ==='
SELECT jsonb_pretty(og_schema('kb', 800));

\echo '=== 008: correctable error ==='
SELECT jsonb_pretty(og_explain_error('kb', 'MATCH (d:Docs) RETURN d'));

\echo '=== 008: cost estimate before running ==='
SELECT og_estimate('kb', 'MATCH (a:Doc), (b:Doc) RETURN a.title, b.title') -> 'advice';

\echo '=== 006: RDF import ==='
SELECT og_create_graph('onto');
SELECT jsonb_pretty(og_load_rdf('onto', $rdf$
@prefix ex: <http://example.org/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

ex:Vehicle a owl:Class .
ex:Car     a owl:Class ; rdfs:subClassOf ex:Vehicle .
ex:EV      a owl:Class ; rdfs:subClassOf ex:Car .
ex:owns    a owl:ObjectProperty ; rdfs:domain ex:Person ; rdfs:range ex:Vehicle .
ex:Person  a owl:Class .

ex:alice a ex:Person ; ex:name "Alice" .
ex:tesla a ex:EV ; ex:model "Model 3"@en .
ex:alice ex:owns ex:tesla .
$rdf$));

\echo '=== 006: OWL class hierarchy became a real type hierarchy ==='
SELECT name, depth, parents FROM og_type_view WHERE graph='onto' ORDER BY lft;

\echo '=== 006: and Cypher can query it, subtypes included ==='
SELECT og_cypher('onto', $$MATCH (v:Vehicle) RETURN v.model AS model$$);
SELECT og_cypher('onto', $$MATCH (p:Person)-[:owns]->(v) RETURN p.name AS owner, v.model AS vehicle$$);

\echo '=== 006: round trip ==='
SELECT left(og_dump_rdf('onto'), 700);
