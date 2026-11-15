UNWIND $ids as id
MATCH (n {id: id})
RETURN n.id as id,
       labels(n) as labels,
       labels(n)[0] as type,
       n.name as name,
       n.description as description,
       n.attachedToId as attachedToId,
       n.attachedToType as attachedToType,
       n.attachedToName as attachedToName
