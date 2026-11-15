MATCH (:_OntologyModel)-[:HAS_CLASS]->(c:_OntologyClass) OPTIONAL MATCH (c)-[:HAS_BINDING]->(b:_OntologyBinding) RETURN DISTINCT c.class_name AS n, b.key_columns AS k
