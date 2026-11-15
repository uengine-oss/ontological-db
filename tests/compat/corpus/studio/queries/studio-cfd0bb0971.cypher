MATCH (n:_Entity) WHERE n.embedding IS NULL
AND NOT any(l IN labels(n) WHERE l IN $virtual) 
RETURN elementId(n) AS eid,
       n.name AS name, n.title AS title,
       substring(toString(n.content), 0, 1500) AS content
LIMIT 500
