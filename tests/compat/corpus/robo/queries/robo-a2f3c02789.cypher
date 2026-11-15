MATCH (ui:UI {id: $uid})
SET ui.figmaNodeId = $fid, ui.figmaFileKey = $fk, ui.updatedAt = datetime(),
    ui.sceneGraph = CASE WHEN $write_sg THEN $sg ELSE ui.sceneGraph END
