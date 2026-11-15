UNWIND $fn_names AS fn
MATCH (f {_owner: 'analyzer'})
WHERE f.name = fn AND (f:FUNCTION OR f:PROCEDURE OR f:METHOD OR f:TRIGGER)
OPTIONAL MATCH (caller)-[:CALLS]->(f)
OPTIONAL MATCH (container)-[:PARENT_OF]->(f)
OPTIONAL MATCH (parent)-[:PARENT_OF]->(container)
WITH fn, f,
     collect(DISTINCT caller.name) AS callers,
     collect(DISTINCT container.name) AS containers,
     collect(DISTINCT parent.name) AS parents
RETURN fn,
       f.summary AS summary,
       [c IN callers WHERE c IS NOT NULL] AS callers,
       head([m IN containers WHERE m IS NOT NULL]) AS container,
       head([p IN parents WHERE p IS NOT NULL]) AS parent
