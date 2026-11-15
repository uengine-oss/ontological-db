MATCH (n {graph_owner: 'analyzer'}) WHERE n.embedded = true RETURN count(n) AS c
