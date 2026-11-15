MERGE (n:Aggregate {id: $id})
ON CREATE SET n.createdAt = datetime(), n.proposalSource = $pid,
              n.key = $key
SET n.updatedAt = datetime()
