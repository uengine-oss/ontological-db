MATCH (ui:UI) WHERE ui.id = $id RETURN ui.sceneGraph AS sg LIMIT 1
