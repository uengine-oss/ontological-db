MATCH (us:UserStory {id: $id})
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(cmd:Command)
OPTIONAL MATCH (agg:Aggregate)-[:HAS_COMMAND]->(cmd)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (evt)-[:TRIGGERS]->(pol:Policy)
WITH collect(DISTINCT cmd.id) + collect(DISTINCT agg.id)
   + collect(DISTINCT evt.id) + collect(DISTINCT pol.id) AS ids
RETURN [x IN ids WHERE x IS NOT NULL] AS impacted
