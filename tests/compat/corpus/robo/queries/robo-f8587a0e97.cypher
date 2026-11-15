MATCH (bc:BoundedContext) WHERE bc.id IN $bcs
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (bc)-[:HAS_READMODEL]->(rm:ReadModel)
RETURN count(DISTINCT agg) AS aggs, count(DISTINCT cmd) AS cmds,
       count(DISTINCT evt) AS evts, count(DISTINCT rm) AS rms
