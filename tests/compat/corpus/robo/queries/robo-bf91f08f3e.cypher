MATCH (agg {id: $aid})-[:HAS_PROPERTY]->(prop:Property {name: $name})
SET prop.type = $type, prop.updatedAt = datetime()
