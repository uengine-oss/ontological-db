MATCH (a)-[r]-(b)
WHERE a.id IN $ids AND startNode(r).id IS NOT NULL AND endNode(r).id IS NOT NULL
WITH DISTINCT r, startNode(r) AS s, endNode(r) AS e
RETURN collect({
    type: type(r), props: properties(r),
    startId: s.id, endId: e.id,
    startLabels: labels(s), endLabels: labels(e)
}) AS rels
