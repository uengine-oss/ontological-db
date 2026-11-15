MERGE (n:_Entity:Customer { customer_id: $merge_customer_id })
ON CREATE SET n.created_at = datetime()
SET n.customer_id = $customer_id, n.name = $name, n.grade = $grade,
    n.updated_at = datetime()
RETURN n
