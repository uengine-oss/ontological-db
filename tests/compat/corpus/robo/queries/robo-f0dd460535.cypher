MATCH (p:Property {id: $id})
DETACH DELETE p
