MATCH (p:Process)
WHERE p.session_id IS NULL AND p.proc_id IS NOT NULL
RETURN p.proc_id AS proc_id, p.name AS name,
       p.description AS description, p.purpose AS purpose
