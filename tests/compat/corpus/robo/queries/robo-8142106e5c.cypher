MATCH (ui:UI {id: $ui_id})
SET ui.figmaNodeId = $fid, ui.figmaFileKey = $fk, ui.updatedAt = datetime()
