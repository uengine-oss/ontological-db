MATCH (n)-[:HAS_PROPERTY]->(p:Property)
WHERE n.id IN $ids
RETURN n.id AS nid, collect(p {.*}) AS props
