UNWIND $ids as eid
MATCH (prop:Property {parentId: eid})
WITH eid, prop
ORDER BY coalesce(prop.isKey, false) DESC,
         coalesce(prop.isForeignKey, false) DESC,
         prop.name ASC
WITH eid, collect({
    name: prop.name,
    type: prop.type,
    description: prop.description,
    isKey: coalesce(prop.isKey, false),
    isForeignKey: coalesce(prop.isForeignKey, false),
    isRequired: coalesce(prop.isRequired, false)
}) as properties
RETURN eid as elementId, properties
