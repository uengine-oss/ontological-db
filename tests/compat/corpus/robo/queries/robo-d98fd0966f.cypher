MATCH (n:Proposal)
WHERE n.id STARTS WITH 'PRO-'
RETURN max(toInteger(substring(n.id, 4))) AS maxNum
