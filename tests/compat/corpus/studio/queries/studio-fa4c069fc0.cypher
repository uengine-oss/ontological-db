MATCH (n:_Entity:`Customer`) WHERE n.`grade` IS NOT NULL RETURN DISTINCT toString(n.`grade`) AS v
