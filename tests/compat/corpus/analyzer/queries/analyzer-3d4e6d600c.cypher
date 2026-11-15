MATCH (s:TABLE)-[r]->(t:TABLE)
WHERE elementId(r) IN $ids
DELETE r
