MATCH (ui:UI)-[:ATTACHED_TO]->(cmd:Command {id: $cmd_id})
RETURN ui {.*} AS ui
