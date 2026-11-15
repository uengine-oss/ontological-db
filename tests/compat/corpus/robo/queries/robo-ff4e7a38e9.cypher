MATCH (tgt:Property {id: $target_id})
RETURN tgt.isKey as isKey
