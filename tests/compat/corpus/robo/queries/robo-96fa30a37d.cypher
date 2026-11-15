MATCH (n:RequirementChange) WITH count(n) AS cnt DETACH DELETE (n) RETURN cnt
