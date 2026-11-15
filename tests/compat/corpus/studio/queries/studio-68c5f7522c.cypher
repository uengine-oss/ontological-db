MATCH (n:_Entity) WHERE n.title CONTAINS '키워드' OR n.content CONTAINS '키워드'
RETURN labels(n), n.number, n.title, substring(toString(n.content), 0, 300) LIMIT 3
