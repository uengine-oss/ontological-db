MATCH (ui:UI {id: $id})
RETURN ui.figmaFileKey as fileKey,
       ui.figmaNodeId as nodeId,
       ui.updatedAt as updatedAt
