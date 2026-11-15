MATCH (t:BpmTask {session_id: $sid})-[rel:REALIZED_BY]->(rule:Rule {session_id: $sid})
WITH rule.id AS rule_id,
     collect({
         task_id: t.id,
         process_id: t.process_id,
         score: rel.confidence,
         rationale: rel.rationale
     }) AS claims
WHERE size(claims) > 1
RETURN rule_id, claims
