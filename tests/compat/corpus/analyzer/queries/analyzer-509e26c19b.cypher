MATCH (n {graph_owner: 'analyzer'}) WHERE n.embedded = true WITH size(n.search_embedding) AS d RETURN d AS dim, count(*) AS c ORDER BY dim
