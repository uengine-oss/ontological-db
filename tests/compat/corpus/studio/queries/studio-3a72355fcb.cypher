MATCH (n) WHERE n:Supplier AND NOT n:PurchaseOrder RETURN n.name AS name LIMIT 100
