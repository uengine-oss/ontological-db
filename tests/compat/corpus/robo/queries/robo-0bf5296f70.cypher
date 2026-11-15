MATCH (ui:UI {id: $id})
OPTIONAL MATCH (ui)-[r:ATTACHED_TO]->()
DELETE r
