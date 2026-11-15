MATCH (pol:Policy {id: $policy_id})
OPTIONAL MATCH (evt:Event)-[:TRIGGERS]->(pol)
OPTIONAL MATCH (pol)-[:INVOKES]->(cmd:Command)
OPTIONAL MATCH (cmd)-[:EMITS]->(target_evt:Event)
OPTIONAL MATCH (agg:Aggregate)-[:HAS_COMMAND]->(cmd)
WITH pol,
     collect(DISTINCT evt.id) as trigger_event_ids_all,
     collect(DISTINCT evt.name) as trigger_event_names_all,
     collect(DISTINCT cmd.id) as cmd_ids_all,
     collect(DISTINCT cmd.name) as cmd_names_all,
     collect(DISTINCT target_evt.id) as target_evt_ids_all,
     collect(DISTINCT target_evt.name) as target_evt_names_all,
     collect(DISTINCT agg.id) as agg_ids_all,
     collect(DISTINCT agg.name) as agg_names_all
RETURN trigger_event_ids_all,
       trigger_event_names_all,
       cmd_ids_all,
       cmd_names_all,
       target_evt_ids_all,
       target_evt_names_all,
       agg_ids_all,
       agg_names_all
