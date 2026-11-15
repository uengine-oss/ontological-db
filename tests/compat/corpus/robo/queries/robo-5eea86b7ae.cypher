MATCH (:BoundedContext {id:$bc})-[:HAS_READMODEL]->(rm:ReadModel) RETURN rm.id AS id, coalesce(rm.displayName,rm.name) AS name, rm.description AS description
