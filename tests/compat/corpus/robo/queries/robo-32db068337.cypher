MATCH (n:RequirementChange)
WHERE n.id STARTS WITH 'CHG-'
RETURN max(toInteger(substring(n.id, 4))) AS maxNum
