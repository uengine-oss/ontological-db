MATCH (n:_Entity) WHERE n._source_id IN $ids RETURN n._source_id AS id
