MATCH (bc:BoundedContext {id: $id})
RETURN bc.id AS id, bc.name AS name, bc.description AS description,
       bc.purpose AS purpose, bc.domainType AS domainType,
       bc.businessModel AS businessModel, bc.evolution AS evolution
