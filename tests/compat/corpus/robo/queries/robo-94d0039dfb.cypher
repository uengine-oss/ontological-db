MATCH (a {id: $s}), (b {id: $e})
MERGE (a)-[rel:`HAS_AGGREGATE`]->(b)
SET rel = $props
RETURN count(rel) AS c
