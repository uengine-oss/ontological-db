UNWIND $items AS item
MATCH (node {id: item.id, graph_owner: 'analyzer'})
SET node.component_id = item.component_id,
    node.component_name = item.component_name
