MATCH (m:_OntologyModel)-[:HAS_CLASS]->(c:_OntologyClass)-[:HAS_ACTION]->(a:_OntologyBehavior {behavior_type:'action'}) RETURN a AS a, c.class_name AS class_name, m.schema_id AS schema_id
