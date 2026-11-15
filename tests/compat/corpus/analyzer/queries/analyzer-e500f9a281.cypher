MATCH (n {graph_owner:$graph_owner}) WHERE n.id IN $ids AND n.search_embedding IS NOT NULL RETURN n.id AS id, n.search_embedding AS v
