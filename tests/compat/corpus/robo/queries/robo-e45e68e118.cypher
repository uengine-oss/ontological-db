MATCH (ui:UI)-[:ATTACHED_TO]->(cmd:Command)
WHERE cmd.id IN $cmd_ids
RETURN cmd.id as commandId,
       ui {.id, .name, .displayName, .description, .template} as ui
