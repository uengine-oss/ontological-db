MATCH (cmd:Command {id: $id})<-[:HAS_COMMAND]-(agg:Aggregate)
RETURN agg.name as aggName, agg.rootEntity as rootEntity
