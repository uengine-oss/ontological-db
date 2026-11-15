MATCH (n)
WHERE n.graph_owner IS NULL
  AND (
    any(label IN labels(n) WHERE label IN $stable_labels)
    OR 'EXTERNAL' IN labels(n)
    OR (n.file_path IS NOT NULL AND n.start_line IS NOT NULL AND n.end_line IS NOT NULL)
  )
SET n.graph_owner = $graph_owner
RETURN count(n) AS migrated
