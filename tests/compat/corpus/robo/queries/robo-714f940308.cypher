MATCH (bc:BoundedContext {id: $context_id})-[:HAS_UI]->(ui:UI)
RETURN ui {.id, .name, .displayName, .description, .template, .attachedToId, .attachedToType, .attachedToName, .userStoryId} as ui
ORDER BY ui.name
