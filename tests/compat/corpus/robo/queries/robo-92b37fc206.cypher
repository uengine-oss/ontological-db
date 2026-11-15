MATCH (parent)-[:HAS_PROPERTY]->(tp:Property {name: $tprop}) WHERE parent.name = $tkey OR parent.key = $tkey OR toLower(parent.name) = toLower($tkey) RETURN tp.id AS id LIMIT 1
