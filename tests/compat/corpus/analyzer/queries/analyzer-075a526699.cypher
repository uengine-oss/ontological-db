UNWIND $items AS item MATCH (n:RULE {id: item.id, graph_owner: 'analyzer'}) SET n.embedding = item.vector
