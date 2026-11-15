MATCH (n:UI {id: $id}) RETURN n.sceneGraph as sg, n.name as name, n.displayName as dn
