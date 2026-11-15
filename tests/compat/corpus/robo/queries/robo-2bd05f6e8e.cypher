MATCH (bc:BoundedContext {id:$bcId})
MERGE (bc)-[:HAS_CONSTITUTION]->(c:Constitution {id:$cid})
SET c.scope='BOUNDED_CONTEXT', c.updatedAt=$updatedAt,
    c.strategicMemory = $mem
