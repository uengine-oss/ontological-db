MATCH (from_n:_Entity) WHERE elementId(from_n) = $from_entity_id
MATCH (to_n:_Entity) WHERE elementId(to_n) = $to_entity_id
MERGE (from_n)-[r:PLACED]->(to_n)
ON CREATE SET r.created_at = datetime()
SET r.updated_at = datetime(), r.channel = $channel
RETURN from_n, r, to_n
