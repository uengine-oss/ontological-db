MATCH (routine)-[:PARENT_OF*]->(statement)
WHERE routine._owner = $owner AND statement._owner = $owner
  AND routine.name = $procedure_name
   AND routine.file_path = $file_path
RETURN statement.start_line AS start_line,
       statement.end_line AS end_line,
       head(labels(statement)) AS statement_type,
       statement.summary AS summary
ORDER BY statement.start_line
