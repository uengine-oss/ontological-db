CALL db.index.vector.queryNodes('analyzer_search_vec', $k, $qv) YIELD node, score WITH node, score WHERE node.id IS NOT NULL AND node.graph_owner = $graph_owner RETURN node.id AS id, score AS score
