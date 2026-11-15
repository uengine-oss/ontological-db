MATCH (:BoundedContext {id:$bc})-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(c:Command) RETURN c.id AS id, coalesce(c.displayName,c.name) AS name, c.description AS description
