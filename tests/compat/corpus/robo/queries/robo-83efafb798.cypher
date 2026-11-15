MATCH (t:BpmTask {session_id: $sid})-[link:REALIZED_BY]->(r:Rule {session_id: $sid})
RETURN t.id AS task_id, r.id AS rule_id,
       link.confidence AS confidence, link.method AS method, link.reviewed AS reviewed,
       link.rationale AS rationale, link.evidence_refs AS evidence_refs,
       link.evidence_path AS evidence_path, link.agent_verdict AS agent_verdict,
       coalesce(link.evidence_role, 'primary') AS evidence_role,
       link.evidence_role_rationale AS evidence_role_rationale
