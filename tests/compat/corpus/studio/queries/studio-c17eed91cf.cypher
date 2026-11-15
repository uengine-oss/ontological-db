MATCH (n:_Entity) WHERE any(l IN labels(n) WHERE l IN $classes) RETURN n._source_id AS key, n.name AS name, labels(n) AS labels
