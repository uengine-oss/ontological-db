MATCH (m:CQRSMapping {id: $mapping_id})
DETACH DELETE m
RETURN count(m) as deleted
