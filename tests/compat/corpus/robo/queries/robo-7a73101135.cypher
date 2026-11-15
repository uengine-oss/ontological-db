MATCH (ui:UI {id: $id}) RETURN ui.sceneGraph as sg, ui.name as name, ui.displayName as dn, ui.figmaNodeId as fid
