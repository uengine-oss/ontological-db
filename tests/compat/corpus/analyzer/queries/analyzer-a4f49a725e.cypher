MATCH (n)
WHERE n.graph_owner = 'analyzer'
  AND n.summary IS NOT NULL AND n.summary <> ''
  AND NOT n:FILE AND NOT n:MODULE AND NOT n:PACKAGE
OPTIONAL MATCH (n)-[rw:READS|WRITES|AFFECTS_TABLE]->(t:TABLE {graph_owner: 'analyzer'})
RETURN n.id AS id,
       coalesce(n.type, head([l IN labels(n) WHERE l <> 'ANALYZER_OWNED'])) AS label,
       coalesce(n.name, '') AS name,
       coalesce(n.stereotype, '') AS stereotype,
       n.summary AS summary,
       coalesce(n.search_text, '') AS search_text,
       type(rw) AS rel_type,
       t.name AS table_name
ORDER BY id
