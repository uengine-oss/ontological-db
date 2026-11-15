UNWIND $items AS item
MERGE (e:ANALYZER_OWNED:EXAMPLE {id: item.id, graph_owner: 'analyzer'})
SET e.given = item.given,
    e.when_ = item.when_,
    e.then_ = item.then_,
    e.graph_owner = 'analyzer'
