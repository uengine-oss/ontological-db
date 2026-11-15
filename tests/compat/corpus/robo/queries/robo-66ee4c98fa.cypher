MATCH (cs:ChangeSet {id: $cs_id})-[r:CONTAINS]->(chg:RequirementChange {id: $chg_id})
DELETE r
SET chg.changeSetId = null
