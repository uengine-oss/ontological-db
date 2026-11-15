MATCH (c:Command)-[:EMITS]->(e:Event {id: $e}) WHERE c.id <> $c RETURN c.id AS id LIMIT 1
