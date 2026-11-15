UNWIND $items AS item
MATCH (node {id: item.id, graph_owner: 'analyzer'})
SET node += item
