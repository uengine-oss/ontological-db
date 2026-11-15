MATCH (n:_Entity:Customer)
WHERE toLower(toString(n.name)) CONTAINS toLower($q)
   OR toLower(toString(n.title)) CONTAINS toLower($q)
   OR toLower(toString(n.content)) CONTAINS toLower($q)
   OR toLower(toString(n._source_id)) CONTAINS toLower($q)
RETURN n LIMIT $limit
