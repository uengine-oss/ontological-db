MATCH (source)-[access:READS|WRITES]->(table:TABLE)
WHERE source._owner = $owner AND table._owner = $owner
  AND (table.name = $table_name OR table._id ENDS WITH $table_name)
OPTIONAL MATCH (routine)-[:PARENT_OF*0..]->(source)
WHERE routine._owner = $owner
  AND any(label IN labels(routine)
          WHERE label IN ['FUNCTION','PROCEDURE','METHOD','TRIGGER'])
WITH source, access, coalesce(routine, source) AS owner_node
RETURN DISTINCT
    coalesce(owner_node.name, owner_node._id) AS procedure_name,
    head(labels(owner_node)) AS procedure_type,
    type(access) AS access_type,
    owner_node.start_line AS start_line,
    owner_node.end_line AS end_line,
    head(labels(source)) AS statement_type,
    source.start_line AS statement_line,
    source.start_line AS evidence_line,
    owner_node.file_path AS file_path,
    owner_node.code_text AS code_text
ORDER BY procedure_name, statement_line
