MATCH (routine)-[hr:HAS_RULE]->(rule:RULE {_owner: 'analyzer'})
WHERE routine._owner = 'analyzer'
  AND (routine:FUNCTION OR routine:PROCEDURE OR routine:METHOD OR routine:TRIGGER)
OPTIONAL MATCH (container {_owner: 'analyzer'})-[:PARENT_OF]->(routine)
OPTIONAL MATCH (rule)-[write:WRITES]->(table:TABLE {_owner: 'analyzer'})
WITH routine, hr, rule, head(collect(DISTINCT container._id)) AS source_container_id,
     collect(DISTINCT {
       table: table.name,
       access: 'WRITE',
       operations: coalesce(write.operations, []),
       op_source: 'SCANNER'
     }) AS raw_writes
RETURN routine._id AS function_id,
       coalesce(routine.name, '') AS function_name,
       coalesce(routine.summary, '') AS function_summary,
       source_container_id,
       rule._id AS analyzer_rule_id,
       coalesce(hr.order, 0) AS rule_order,
       coalesce(rule.condition, '') AS condition,
       coalesce(rule.effects, []) AS effects,
       coalesce(rule.condition_description, '') AS condition_description,
       coalesce(rule.effect_descriptions, []) AS effect_descriptions,
       raw_writes
ORDER BY function_id, rule_order, analyzer_rule_id
