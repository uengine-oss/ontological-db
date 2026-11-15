MATCH (rm:ReadModel {id: $id})
OPTIONAL MATCH (rm)-[:HAS_PROPERTY]->(p:Property)
WITH rm, collect(p.name) as propNames
RETURN rm.name as rmName, rm.description as rmDesc,
       [n IN propNames WHERE n IS NOT NULL | n] as fields
