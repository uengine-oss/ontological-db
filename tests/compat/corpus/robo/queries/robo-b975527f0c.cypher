MATCH (ui:UI)
WHERE ui.displayName = $name OR ui.name = $name
SET ui.figmaFileKey = $file_key,
    ui.figmaNodeId = $figma_node_id,
    ui.updatedAt = datetime()
RETURN ui.id as id, ui.name as name
LIMIT 1
