MATCH (ui:UI {id: $id}) SET ui.template = $tmpl, ui.updatedAt = datetime()
