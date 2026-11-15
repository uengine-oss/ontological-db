MATCH (n:RequirementChange) WHERE n.status = $status
RETURN n {.*} AS change ORDER BY n.createdAt DESC LIMIT $limit
