MERGE (s:_Entity:Customer {_source_id: $s_key})
ON CREATE SET s.created_at = datetime(), s._confidence = $s_conf,
              s._run_id = $run_id, s._evidence_ref = $evidence_ref,
              s._source_system = $s_sys, s._source_key = $s_skey
SET s.updated_at = datetime()
SET s.name = coalesce(s.name, $s_name)
RETURN elementId(s) AS sid
