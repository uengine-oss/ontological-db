MATCH (h:_Entity)-[r]->(t:_Entity) WHERE any(l IN labels(h) WHERE l IN $classes) AND any(l IN labels(t) WHERE l IN $classes) RETURN h._source_id AS h, type(r) AS r, t._source_id AS t
