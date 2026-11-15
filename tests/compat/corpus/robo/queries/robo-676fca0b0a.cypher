MATCH (r:RULE {id: $id})-[:HAS_EXAMPLE]->(e:EXAMPLE) RETURN e.id AS id, coalesce(e.given, '') AS given,   coalesce(e.when_, '') AS when, coalesce(e.then_, '') AS then
