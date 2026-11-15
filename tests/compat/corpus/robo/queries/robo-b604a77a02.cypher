MATCH (ui:UI)
WHERE ui.userStoryId = $usid
RETURN ui.id AS id, ui.name AS name, ui.template AS template,
       ui.sceneGraph AS sceneGraph,
       ui.attachedToType AS attachedToType,
       ui.attachedToName AS attachedToName,
       ui.actor AS actor
ORDER BY ui.name
