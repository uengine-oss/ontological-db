MATCH (t:TABLE {graph_owner: $graph_owner}) WHERE t.id IN $ids RETURN count(t) AS matched
