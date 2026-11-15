UNWIND $items AS item
MERGE (n:ANALYZER_OWNED:RULE {id: item.id, graph_owner: 'analyzer'})
SET n += item,
    n.graph_owner = 'analyzer'
RETURN count(n) AS created
