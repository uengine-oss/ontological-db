MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_READMODEL]->(rm:ReadModel)
RETURN rm
