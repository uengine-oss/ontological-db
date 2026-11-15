MERGE (s:_Entity:Customer {_source_id: $s_key})
ON CREATE SET s.created_at = datetime(), s._confidence = $s_conf,
              s._run_id = $run_id, s._evidence_ref = $evidence_ref,
              s._source_system = $s_sys, s._source_key = $s_skey
SET s.updated_at = datetime()
SET s.name = coalesce(s.name, $s_name)
WITH s
MERGE (o:_Entity:PurchaseOrder {_source_id: $o_key})
ON CREATE SET o.created_at = datetime(), o._confidence = $o_conf,
              o._run_id = $run_id, o._evidence_ref = $evidence_ref,
              o._source_system = $o_sys, o._source_key = $o_skey
SET o.updated_at = datetime()
SET o.name = coalesce(o.name, $o_name)
WITH s, o
MERGE (s)-[r:PLACED]->(o)
ON CREATE SET r.created_at = datetime(), r._confidence = $confidence, r._run_id = $run_id, r._evidence_ref = $evidence_ref
SET r.updated_at = datetime()
RETURN elementId(s) AS sid, elementId(o) AS oid, elementId(r) AS rid
