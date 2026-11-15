MATCH (cs:ChangeSet {id: $id})
OPTIONAL MATCH (cs)-[:CONTAINS]->(chg:RequirementChange)
OPTIONAL MATCH (chg)-[e:EFFECT]->(t)
RETURN cs {.*} AS cs,
       collect(DISTINCT chg {.*}) AS changes
