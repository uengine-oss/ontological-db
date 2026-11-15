MATCH (inv:Invariant {id: $id})
OPTIONAL MATCH (inv)-[:HAS_GWT]->(og:GWT)
DETACH DELETE og
