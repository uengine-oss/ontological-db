MATCH (:BoundedContext {id:$bcId})-[:HAS_CONSTITUTION]->(c:Constitution) RETURN c
