MATCH (bc:BoundedContext {id: $id})
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
WITH collect(DISTINCT agg.id) + collect(DISTINCT cmd.id)
   + collect(DISTINCT evt.id) AS ids
RETURN [x IN ids WHERE x IS NOT NULL] AS ids
