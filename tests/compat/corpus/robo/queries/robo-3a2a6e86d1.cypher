MATCH (agg {id: $aid})-[:HAS_PROPERTY]->(prop:Property {name: $name})
DETACH DELETE prop
