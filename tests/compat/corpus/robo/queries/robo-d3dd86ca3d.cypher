MATCH (bc:BoundedContext {id:$bcId})
MERGE (bc)-[:HAS_CONSTITUTION]->(c:Constitution {id:$cid})
SET c.scope='BOUNDED_CONTEXT', c.raw=$raw, c.updatedAt=$updatedAt, c.principles=$principles
