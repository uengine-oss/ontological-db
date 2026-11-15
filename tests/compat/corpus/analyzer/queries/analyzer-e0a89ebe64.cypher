MATCH (n {graph_owner: 'analyzer'})
WHERE n.domain_terms_json IS NOT NULL AND n.domain_terms_json <> ''
RETURN n.id AS id, n.name AS name, coalesce(n.file_path,'') AS file_path,
       coalesce(n.start_line,0) AS start_line,
       coalesce(n.summary,'') AS summary, coalesce(n.search_text,'') AS search_text,
       n.domain_terms_json AS domain_terms_json
