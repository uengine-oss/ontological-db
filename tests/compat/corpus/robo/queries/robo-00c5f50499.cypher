MATCH (u:UI {id: $uid})
OPTIONAL MATCH (u)<-[:HAS_UI]-(bc:BoundedContext)
RETURN coalesce(u.displayName, u.name, '') AS displayName,
       coalesce(u.description, '') AS description,
       coalesce(bc.displayName, bc.name, '') AS bcName
