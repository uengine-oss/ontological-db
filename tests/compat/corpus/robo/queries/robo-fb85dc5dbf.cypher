MATCH (bc:BoundedContext {id: $bc_id})
MERGE (ui:UI {key: $key})
ON CREATE SET ui.id = randomUUID(),
              ui.createdAt = datetime()
SET ui.key = $key,
    ui.name = $name,
    ui.displayName = $display_name,
    ui.description = $description,
    ui.template = $template,
    ui.sceneGraph = $scene_graph,
    ui.attachedToId = $attached_to_id,
    ui.attachedToType = $attached_to_type,
    ui.attachedToName = $attached_to_name,
    ui.userStoryId = $user_story_id,
    ui.actor = $actor,
    ui.figmaFileKey = $figma_file_key,
    ui.figmaNodeId = $figma_node_id,
    ui.updatedAt = datetime()
MERGE (bc)-[:HAS_UI]->(ui)
RETURN ui {.id, .key, .name, .displayName, .description, .template, .sceneGraph, .attachedToId, .attachedToType, .attachedToName, .userStoryId, .actor, .figmaFileKey, .figmaNodeId} as ui
