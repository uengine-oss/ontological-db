MATCH (n:ChangeSet)
WHERE n.id STARTS WITH 'CS-'
RETURN max(toInteger(substring(n.id, 3))) AS maxNum
