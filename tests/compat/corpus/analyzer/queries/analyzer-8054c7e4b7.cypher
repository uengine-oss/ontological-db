UNWIND $items AS item
MERGE (r:ANALYZER_OWNED:RULE {id: item.id, graph_owner: 'analyzer'})
SET r.statement = item.statement,
    r.graph_owner = 'analyzer'
