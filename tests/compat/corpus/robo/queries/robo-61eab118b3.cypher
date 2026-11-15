MATCH (a:Aggregate {id: $id})-[:HAS_PROPERTY]->(p:Property)
RETURN p.name AS name, p.type AS type, p.description AS description,
       p.isKey AS isKey, p.isForeignKey AS isForeignKey,
       p.mutability AS mutability
ORDER BY coalesce(p.isKey, false) DESC, p.name
