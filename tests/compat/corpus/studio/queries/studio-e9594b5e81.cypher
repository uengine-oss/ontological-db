MATCH (n:`Customer`) WHERE toString(n.`grade`) = $v OR toLower(trim(toString(n.`grade`))) = $lv RETURN DISTINCT toString(n.`grade`) AS value LIMIT $limit
