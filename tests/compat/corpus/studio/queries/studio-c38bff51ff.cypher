MATCH (a:_Entity {_source_id: $from_id})
MATCH (b:_Entity {_source_id: $to_id})
MERGE (a)-[r:PLACED]->(b)
ON CREATE SET r.created_at = datetime()
SET r.updated_at = datetime(), r.channel = $channel
