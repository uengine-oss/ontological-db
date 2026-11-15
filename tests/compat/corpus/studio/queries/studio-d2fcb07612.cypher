MATCH (n:_Entity:`Customer`) WHERE toLower(toString(n.`name`)) CONTAINS toLower($text) OR toLower(toString(n.`email`)) CONTAINS toLower($text) RETURN n, elementId(n) AS _element_id LIMIT $limit
