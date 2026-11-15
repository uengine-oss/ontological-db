MATCH (fk:Property {id: $f}), (tp:Property {id: $t}) MERGE (fk)-[:REFERENCES]->(tp)
