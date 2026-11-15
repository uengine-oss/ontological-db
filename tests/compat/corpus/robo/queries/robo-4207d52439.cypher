UNWIND $rows AS r
MATCH (bc:BoundedContext {id: r.bc_id})
MERGE (ui:UI {key: r.key})
  ON CREATE SET ui.id = randomUUID(),
                ui.createdAt = datetime()
SET ui.key = r.key,
    ui.name = r.name,
    ui.displayName = r.display_name,
    ui.description = r.description,
    ui.template = r.template,
    ui.sceneGraph = r.scene_graph,
    ui.attachedToId = r.attached_to_id,
    ui.attachedToType = r.attached_to_type,
    ui.attachedToName = r.attached_to_name,
    ui.userStoryId = r.user_story_id,
    ui.actor = r.actor,
    ui.figmaFileKey = r.figma_file_key,
    ui.figmaNodeId = r.figma_node_id,
    ui.updatedAt = datetime()
MERGE (bc)-[:HAS_UI]->(ui)
RETURN ui {.id, .key, .name, .displayName, .description, .template, .sceneGraph,
           .attachedToId, .attachedToType, .attachedToName, .userStoryId, .actor,
           .figmaFileKey, .figmaNodeId} AS result
