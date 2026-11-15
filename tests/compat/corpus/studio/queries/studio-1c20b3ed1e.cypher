MATCH (m:_OntologyModel)-[:HAS_CLASS]->(c:_OntologyClass) WHERE m.schema_id = $schema_id RETURN DISTINCT c.class_name AS class_name
