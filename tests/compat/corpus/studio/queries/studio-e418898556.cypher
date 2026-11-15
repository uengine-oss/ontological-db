MATCH (m:_OntologyModel)-[:HAS_CLASS]->(c:_OntologyClass {class_name: $class_name}) -[:HAS_BINDING]->(:_OntologyBinding) RETURN m.name AS schema_name ORDER BY schema_name
