MATCH (f)
WHERE (f:FUNCTION OR f:METHOD)
  AND f.summary IS NOT NULL AND f.summary <> ''
RETURN coalesce(f.function_id, f._id, f.id) AS fqn, f.name AS name,
       f.summary AS summary, f.stereotype AS stereotype
