MATCH (bc:BoundedContext {id: $bc_id})
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (bc)-[:HAS_READMODEL]->(rm:ReadModel)
WITH bc, agg, cmd, evt, rm
WITH bc,
     agg,
     collect(DISTINCT { id: cmd.id, name: cmd.name }) as commands,
     collect(DISTINCT { id: evt.id, name: evt.name }) as events,
     collect(DISTINCT { id: rm.id,  name: rm.name  }) as readmodels
RETURN
    bc.id   as id,
    bc.name as name,
    bc.classification as classification,
    coalesce(bc.version, 0) as version,
    collect(DISTINCT {
        id: agg.id, name: agg.name,
        rootEntity: agg.rootEntity,
        version: coalesce(agg.version, 0)
    }) as aggregates,
    [cmd IN commands   WHERE cmd.id IS NOT NULL] as commands,
    [evt IN events     WHERE evt.id IS NOT NULL] as events,
    [rm  IN readmodels WHERE rm.id  IS NOT NULL] as readmodels
