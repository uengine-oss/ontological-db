MERGE (n:_Entity:Customer { customer_id: $merge_customer_id })
ON CREATE SET n.created_at = datetime()
SET n.name = $name, n.grade = $grade, n._source_id = $_source_id, n._parent_source_id = $_parent_source_id, n.updated_at = datetime()
RETURN n
