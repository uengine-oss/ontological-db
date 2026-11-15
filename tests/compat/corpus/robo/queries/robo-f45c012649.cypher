MATCH (ui:UI {id: $id}) SET ui.sceneGraph = $sg, ui.updatedAt = datetime()
