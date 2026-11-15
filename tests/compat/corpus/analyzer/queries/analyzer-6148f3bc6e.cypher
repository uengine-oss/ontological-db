MATCH (m {graph_owner: 'analyzer'})
WHERE (m:MODULE OR m:FILE) AND m.summary IS NOT NULL
RETURN m.id AS id, coalesce(m.file_path,'') AS file_path,
       coalesce(m.start_line,0) AS start_line, coalesce(m.end_line,0) AS end_line,
       m.summary AS summary
