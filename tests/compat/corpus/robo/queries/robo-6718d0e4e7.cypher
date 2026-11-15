MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
WITH bc,
     collect(DISTINCT {
         id: agg.id, name: agg.name,
         version: coalesce(agg.version, 0)
     }) AS aggregates
RETURN
    bc.id AS id,
    bc.name AS name,
    bc.classification AS classification,
    bc.domainType AS domainType,
    coalesce(bc.version, 0) AS version,
    [a IN aggregates WHERE a.id IS NOT NULL] AS aggregates
ORDER BY bc.name
