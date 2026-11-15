MATCH (agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
WITH agg, collect(DISTINCT cmd {.id, .name}) AS commands,
     collect(DISTINCT evt.name) AS events
RETURN agg.id AS aggId, agg.key AS aggKey, agg.name AS aggName,
       agg.rootEntity AS rootEntity, commands, events
ORDER BY agg.name
