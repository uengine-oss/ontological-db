MATCH (c)-[:PARENT_OF]->(f)
WHERE (f:FUNCTION OR f:METHOD OR f:PROCEDURE OR f:TRIGGER)
RETURN coalesce(f.function_id, f._id, f.id) AS child,
       coalesce(c.module_id, c.id) AS parent
