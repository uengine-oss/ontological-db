MATCH (bc:BoundedContext {id: $bc_id})
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
RETURN
    bc.id   as id,
    bc.name as name,
    bc.classification as classification,
    coalesce(bc.version, 0) as version,
    collect(DISTINCT {
        id: agg.id, name: agg.name,
        rootEntity: agg.rootEntity,
        version: coalesce(agg.version, 0)
    }) as aggregates
