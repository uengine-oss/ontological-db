MATCH (src:Property {id: $source_id})
MATCH (tgt:Property {id: $target_id})
SET src.isForeignKey = true
MERGE (src)-[:REFERENCES]->(tgt)
