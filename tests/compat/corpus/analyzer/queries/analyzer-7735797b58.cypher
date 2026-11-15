UNWIND $items AS item
MATCH (node {id: item.id, graph_owner: 'analyzer'})
REMOVE node.domain_terms, node.domain_terms_json
SET node += item
