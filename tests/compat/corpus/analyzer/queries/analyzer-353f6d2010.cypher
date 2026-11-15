UNWIND $items AS item MATCH (n {id: item.id, graph_owner: 'analyzer'}) SET n.search_embedding = item.vector, n.embedded = true
