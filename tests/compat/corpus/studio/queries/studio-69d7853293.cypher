MATCH (c:_OntologyClass)-[:HAS_BINDING]->(b:_OntologyBinding) RETURN c.class_name AS n, b.virtual_node AS v
