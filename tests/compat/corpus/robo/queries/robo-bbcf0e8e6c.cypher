MATCH (cs:ChangeSet {id: $cs_id})-[:CONTAINS]->(chg:RequirementChange)
WHERE chg.status = $from_status
WITH chg
SET chg.status = $to_status
WITH chg,
     apoc.convert.fromJsonList(COALESCE(chg.statusHistory, '[]')) AS hist
SET chg.statusHistory = apoc.convert.toJson(
    hist + [{fromStatus: $from_status, toStatus: $to_status, at: datetime(), actor: $actor}]
)
