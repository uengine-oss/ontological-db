MATCH (us:UserStory {id: $usid})-[:SOURCED_FROM]->(sr:Rule)
RETURN sr.id AS seq,
       sr.title AS title,
       coalesce(sr.given, '') AS given,
       coalesce(sr.`when`, '') AS wh,
       coalesce(sr.`then`, '') AS th,
       sr.source_function_id AS function_id,
       sr.source_function AS function_name,
       coalesce(sr.write_effects_json, '[]') AS write_effects_json
ORDER BY sr.id
