MATCH (n:RequirementChange)
WHERE n.id <> $id
  AND n.status = 'APPROVED'
  AND n.createdAt < $createdAt
RETURN n {.*} AS n
ORDER BY n.createdAt ASC
