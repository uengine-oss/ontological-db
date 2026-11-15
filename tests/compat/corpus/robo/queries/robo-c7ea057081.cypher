MATCH (agg:Aggregate {id: $aggregate_id})
MATCH (evt:Event {name: $event_name})
MERGE (agg)-[r:SCOPE_EVENT]->(evt)
ON CREATE SET r.createdAt = datetime()
RETURN evt.id AS id
LIMIT 1
