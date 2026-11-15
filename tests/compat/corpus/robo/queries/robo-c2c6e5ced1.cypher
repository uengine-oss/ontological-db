UNWIND $rows AS r
MATCH (cmd:Command {id: r.cmd_id})
MATCH (evt:Event {id: r.evt_id})
MERGE (cmd)-[em:EMITS]->(evt)
  ON CREATE SET em.isGuaranteed = coalesce(r.is_guaranteed, true)
SET em.isGuaranteed = coalesce(em.isGuaranteed, coalesce(r.is_guaranteed, true))
RETURN {cmd_id: cmd.id, evt_id: evt.id} AS result
