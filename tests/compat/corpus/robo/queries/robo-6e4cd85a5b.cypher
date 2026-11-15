MATCH (inv:Invariant {id: $iid})-[r:VERIFIED_BY]->(cmd:Command {id: $cid})
DELETE r
RETURN count(r) AS removed
