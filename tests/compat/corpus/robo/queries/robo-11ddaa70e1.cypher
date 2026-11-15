UNWIND $functions AS requested
MATCH (routine {_owner: 'analyzer'})
WHERE (routine:FUNCTION OR routine:PROCEDURE OR routine:METHOD OR routine:TRIGGER)
  AND ((requested.id IS NOT NULL AND routine._id = requested.id)
       OR (requested.id IS NULL AND routine.name = requested.name))
OPTIONAL MATCH (routine)-[:PARENT_OF*0..]->(_read)-[:READS]->(read_table:TABLE)
OPTIONAL MATCH (routine)-[:PARENT_OF*0..]->(_write)-[:WRITES]->(write_table:TABLE)
OPTIONAL MATCH (caller)-[:CALLS]->(routine)
OPTIONAL MATCH (routine)-[:CALLS]->(callee)
OPTIONAL MATCH (container)-[:PARENT_OF]->(routine)
OPTIONAL MATCH (parent)-[:PARENT_OF]->(container)
WITH requested, routine,
     collect(DISTINCT read_table.name) AS reads,
     collect(DISTINCT write_table.name) AS writes,
     collect(DISTINCT caller.name) AS callers,
     collect(DISTINCT callee.name) AS callees,
     collect(DISTINCT container.name) AS containers,
     collect(DISTINCT parent.name) AS parents
RETURN requested.id AS requested_id,
       requested.name AS requested_name,
       routine.summary AS summary,
       [value IN reads WHERE value IS NOT NULL] AS reads_tables,
       [value IN writes WHERE value IS NOT NULL] AS writes_tables,
       [value IN callers WHERE value IS NOT NULL] AS callers,
       [value IN callees WHERE value IS NOT NULL] AS callees,
       head([value IN containers WHERE value IS NOT NULL]) AS parent_container,
       head([value IN parents WHERE value IS NOT NULL]) AS container_parent
