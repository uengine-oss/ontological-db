MATCH (n:_Entity)
WITH [l IN labels(n) WHERE l <> '_Entity'] AS entity_labels
UNWIND entity_labels AS label
RETURN label AS class, count(*) AS count
ORDER BY count DESC
