UNWIND $items as it
MATCH (src:Property {id: it.srcId})
WITH src, it
CALL {
  WITH it
  OPTIONAL MATCH (pAgg:Aggregate {key: it.tgtKey}) WHERE it.tgtType = 'Aggregate'
  OPTIONAL MATCH (pCmd:Command {key: it.tgtKey}) WHERE it.tgtType = 'Command'
  OPTIONAL MATCH (pEvt:Event {key: it.tgtKey}) WHERE it.tgtType = 'Event'
  OPTIONAL MATCH (pRm:ReadModel {key: it.tgtKey}) WHERE it.tgtType = 'ReadModel'
  RETURN coalesce(pAgg, pCmd, pEvt, pRm) as parent
}
WITH src, it, parent
OPTIONAL MATCH (tgt:Property {parentType: it.tgtType, parentId: parent.id, name: it.tgtProp})
WITH src, it, parent, tgt,
     (parent IS NOT NULL) as parentFound,
     (tgt IS NOT NULL) as tgtFound,
     (coalesce(tgt.isKey, false) = true) as tgtIsKey
FOREACH (_ IN CASE WHEN parentFound AND tgtFound AND tgtIsKey THEN [1] ELSE [] END |
  SET src.isForeignKey = true
  MERGE (src)-[:REFERENCES]->(tgt)
)
RETURN
  count(*) as scanned,
  sum(CASE WHEN parentFound THEN 1 ELSE 0 END) as parentFound,
  sum(CASE WHEN tgtFound THEN 1 ELSE 0 END) as tgtFound,
  sum(CASE WHEN parentFound AND tgtFound AND tgtIsKey THEN 1 ELSE 0 END) as created,
  sum(CASE WHEN NOT parentFound THEN 1 ELSE 0 END) as skipped_parent_missing,
  sum(CASE WHEN parentFound AND NOT tgtFound THEN 1 ELSE 0 END) as skipped_target_missing,
  sum(CASE WHEN parentFound AND tgtFound AND NOT tgtIsKey THEN 1 ELSE 0 END) as skipped_not_key
