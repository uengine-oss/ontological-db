UNWIND $items AS item
MERGE (q:ANALYZER_OWNED:QUESTION {id: item.id, graph_owner: 'analyzer'})
SET q.text = item.text,
    q.reason = item.reason,
    q.graph_owner = 'analyzer'
