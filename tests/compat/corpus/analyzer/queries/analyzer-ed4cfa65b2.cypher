UNWIND $items AS item
MERGE (p:ANALYZER_OWNED:PACKAGE {id: item.id, graph_owner: 'analyzer'})
SET p.name = item.name,
    p.graph_owner = 'analyzer'
RETURN count(p) AS created
