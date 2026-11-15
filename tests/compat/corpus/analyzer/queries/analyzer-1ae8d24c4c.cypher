MATCH (c:COLUMN {graph_owner: $graph_owner}) WHERE c.id IN $ids RETURN count(c) AS matched
