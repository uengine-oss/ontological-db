UNWIND $pairs AS pair
MATCH (t:BpmTask {id: pair.task_id, session_id: $sid})
      -[:REALIZED_BY]->(sh:Rule {session_id: $sid})
RETURN pair.us_id AS us_id,
       sh.id AS rule_id,
       sh.title AS title,
       sh.source_function AS source_function,
       sh.source_function_id AS source_function_id,
       sh.source_rule_id AS source_rule_id,
       coalesce(sh.given, '') AS given,
       coalesce(sh.`when`, '') AS when_,
       coalesce(sh.`then`, '') AS then_,
       coalesce(sh.write_effects_json, '[]') AS write_effects_json
