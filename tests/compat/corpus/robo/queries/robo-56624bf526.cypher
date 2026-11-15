MATCH (ui:UI {id: $id})
RETURN ui.id as id, ui.name as name, ui.description as description,
       ui.attachedToName as attachedToName, ui.attachedToId as attachedToId,
       ui.attachedToType as attachedToType, ui.template as template
