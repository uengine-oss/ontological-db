MATCH (n:_Entity:Customer) RETURN count(*) AS count, sum(n.credit_limit) AS total_credit
