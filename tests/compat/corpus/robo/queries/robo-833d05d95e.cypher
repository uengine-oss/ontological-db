OPTIONAL MATCH (bc:BoundedContext)
  WHERE bc.name = $q OR bc.key = $q OR bc.id = $q
OPTIONAL MATCH (agg:Aggregate)
  WHERE agg.name = $q OR agg.key = $q OR agg.id = $q
WITH collect(DISTINCT bc) + collect(DISTINCT agg) as nodes
UNWIND nodes as n
WITH n WHERE n IS NOT NULL
RETURN
    n.id     as id,
    n.name   as name,
    head([lbl IN labels(n) WHERE lbl IN ['BoundedContext','Aggregate']]) as kind,
    coalesce(n.classification, null) as classification,
    coalesce(n.version, 0) as version
