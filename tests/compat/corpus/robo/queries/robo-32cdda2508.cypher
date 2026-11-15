MATCH (:BoundedContext {id:$bcId})-[:HAS_CONSTITUTION]->(c:Constitution) DETACH DELETE c
