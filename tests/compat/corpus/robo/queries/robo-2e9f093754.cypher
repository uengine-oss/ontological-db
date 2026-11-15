MATCH (n) WHERE n.id IN $ids DETACH DELETE n RETURN count(n) AS c
