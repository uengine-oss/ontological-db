MATCH (n:ChangeSet) WITH count(n) AS cnt DETACH DELETE (n) RETURN cnt
