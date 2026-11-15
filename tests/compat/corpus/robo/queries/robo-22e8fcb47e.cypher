MATCH (bc:BoundedContext {id: $node_id})-[:HAS_READMODEL]->(rm:ReadModel)
RETURN rm
