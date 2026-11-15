UNWIND $ids AS uid
OPTIONAL MATCH (u:UI {id: uid})
RETURN uid AS id, (u IS NOT NULL) AS present
