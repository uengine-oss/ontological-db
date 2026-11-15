MATCH (p:Process {proc_id: $pid})-[:HAS_TASK]->(t:Task)
OPTIONAL MATCH (t)-[:PERFORMED_BY]->(r:Role)
RETURN t.task_id AS task_id, t.name AS name,
       t.description AS description, t.instruction AS instruction,
       t.order AS order_, t.task_type AS task_type,
       r.role_id AS role_id, r.name AS role_name,
       r.persona_hint AS role_hint
ORDER BY toInteger(coalesce(t.order, '0')), t.task_id
