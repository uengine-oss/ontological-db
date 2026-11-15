MATCH (bc:BoundedContext {id: $bc_id})-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (agg)-[:HAS_PROPERTY]->(p:Property)
RETURN
    agg.id   AS id,
    agg.name AS name,
    'Aggregate' AS kind,
    coalesce(agg.version, 0) AS version,
    collect({ name: p.name, type: p.type }) AS properties
