MATCH (evt:Event {id: $evt_id})-[:HAS_PROPERTY]->(p:Property) RETURN p.name as name, p.type as type ORDER BY p.name
