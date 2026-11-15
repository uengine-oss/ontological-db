MATCH (a:Aggregate {id: $agg_id})-[:HAS_COMMAND]->(:Command)-[:EMITS]->(e:Event)
WITH DISTINCT e
RETURN e {.id, .key, .name, .displayName, .version, .schema, .payload, .description, .sequence} AS evt
ORDER BY e.name
