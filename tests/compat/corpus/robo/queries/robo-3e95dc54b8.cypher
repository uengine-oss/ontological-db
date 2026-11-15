MATCH (agg:Aggregate {id: $agg_id})<-[:HAS_AGGREGATE]-(bc:BoundedContext)
RETURN bc.id as bc_id
LIMIT 1
