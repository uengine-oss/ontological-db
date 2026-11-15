MATCH (:_OntologyModel {schema_id: $schema_id})-[:HAS_CLASS]->(c:_OntologyClass) RETURN c.class_name AS class_name
