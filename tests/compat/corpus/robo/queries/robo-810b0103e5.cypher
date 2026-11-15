MATCH (n:Aggregate {proposalSource: $pid}) RETURN count(n) AS c
