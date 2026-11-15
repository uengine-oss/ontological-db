MATCH (c)-[:PARENT_OF]->(m)
WHERE (m:MODULE OR m:FILE OR m:CLASS OR m:INTERFACE OR m:RECORD)
RETURN coalesce(m.module_id, m.id) AS child,
       coalesce(c.module_id, c.id) AS parent
