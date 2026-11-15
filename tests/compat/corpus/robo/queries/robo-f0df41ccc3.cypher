MATCH (agg:Aggregate)<-[:HAS_AGGREGATE]-(bc:BoundedContext)
WHERE toLower(agg.name) = toLower($hint)
   OR toLower(agg.name) STARTS WITH toLower($hint)
RETURN bc.id as bc_id, agg.id as agg_id, agg.name as agg_name
ORDER BY 
  CASE WHEN toLower(agg.name) = toLower($hint) THEN 0 ELSE 1 END
LIMIT 1
