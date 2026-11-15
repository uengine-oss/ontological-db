MATCH (agg:Aggregate {id: $agg_id})-[:SCOPE_EVENT]->(evt:Event)
RETURN evt.name AS name, evt.displayName AS displayName
UNION
MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_EVENT]->(evt:Event)
RETURN evt.name AS name, evt.displayName AS displayName
