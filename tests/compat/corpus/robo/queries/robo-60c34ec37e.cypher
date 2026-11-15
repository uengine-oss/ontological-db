MATCH (tp:Property {name: $tprop, isKey: true}) RETURN tp.id AS id LIMIT 1
