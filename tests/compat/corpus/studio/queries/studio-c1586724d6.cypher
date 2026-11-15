MATCH (po:PurchaseOrder)-[:ORDERED_FROM]->(s:Supplier) RETURN s.name AS supplier, count(po) AS orders ORDER BY supplier LIMIT 100
