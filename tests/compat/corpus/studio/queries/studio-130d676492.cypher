MATCH (:_OntologyModel)-[:HAS_CLASS]->(c:_OntologyClass) OPTIONAL MATCH (c)-[:HAS_BINDING]->(b:_OntologyBinding) RETURN c.class_name AS class_name, b.key_columns AS key_columns
