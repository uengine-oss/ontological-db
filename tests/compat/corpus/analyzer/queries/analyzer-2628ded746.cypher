MATCH (n)
WHERE n.graph_owner = 'analyzer'
  AND n.summary IS NOT NULL AND n.summary <> ''
  AND NOT n:FILE AND NOT n:MODULE AND NOT n:PACKAGE
RETURN n.id AS id, coalesce(n.type, head([l IN labels(n) WHERE l <> 'ANALYZER_OWNED'])) AS label,
       coalesce(n.name,'') AS name, n.summary AS summary,
       coalesce(n.code_text,'') AS code_text,
       coalesce(n.search_text,'') AS search_text,
       coalesce(n.stereotype,'') AS stereotype,
       coalesce(n.logical_name,'') AS logical_name
ORDER BY id
