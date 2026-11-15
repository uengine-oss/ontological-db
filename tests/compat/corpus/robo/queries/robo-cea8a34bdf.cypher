MATCH (f:Feature {id: $fid})
OPTIONAL MATCH (us:UserStory {id: $usid})-[:IMPLEMENTS]->(curBc:BoundedContext)
RETURN f.boundedContextId AS targetBcId, curBc.id AS currentBcId,
       us.id AS usExists
