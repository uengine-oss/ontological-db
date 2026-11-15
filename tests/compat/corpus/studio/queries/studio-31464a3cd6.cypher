MATCH (n:`Customer`) WHERE n.`grade` IS NOT NULL RETURN DISTINCT toString(n.`grade`) AS value LIMIT $limit
