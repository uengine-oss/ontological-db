UNWIND $ids AS id
MATCH (node {id: id, graph_owner: 'analyzer'})
RETURN node.id AS id
