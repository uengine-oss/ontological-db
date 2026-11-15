MATCH (ui:UI)
WHERE ui.displayName = $name OR ui.name = $name
SET ui.figmaNodeId = $fid, ui.figmaFileKey = $fk, ui.updatedAt = datetime(),
    ui.sceneGraph = CASE WHEN $write_sg THEN $sg ELSE ui.sceneGraph END
