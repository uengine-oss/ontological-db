MATCH (n:_Entity)
WHERE any(lbl IN labels(n) WHERE lbl IN $classes)
RETURN elementId(n) AS id,
       [l IN labels(n) WHERE l <> '_Entity'] AS labels,
       properties(n) AS properties
LIMIT $limit
