MATCH (p:Proposal)
WHERE p.status = $status
RETURN p {.*} AS p
ORDER BY p.createdAt DESC
SKIP $offset LIMIT $limit
