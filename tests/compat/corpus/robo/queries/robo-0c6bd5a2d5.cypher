MATCH (m)
WHERE (m:MODULE OR m:FILE OR m:CLASS OR m:INTERFACE OR m:RECORD)
  AND m.summary IS NOT NULL AND m.summary <> ''
RETURN coalesce(m.module_id, m.id) AS fqn, m.name AS name,
       m.summary AS summary, m.stereotype AS stereotype
