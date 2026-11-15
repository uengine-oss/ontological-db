MATCH (ui:UI {id: $ui_id})
SET ui.sceneGraph = $sg,
    ui.figmaFileKey = $file_key,
    ui.figmaNodeId = $figma_node_id,
    ui.updatedAt = datetime()
RETURN ui {.id, .name, .figmaFileKey, .figmaNodeId} as ui
