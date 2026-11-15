UNWIND $parent_ids as pid
MATCH (prop:Property {parentId: pid})
WITH pid, prop
ORDER BY coalesce(prop.isKey, false) DESC,
         coalesce(prop.isForeignKey, false) DESC,
         prop.name ASC
WITH pid, collect(prop {
    .id, .name, .displayName, .type, .description,
    .isKey, .isForeignKey, .isRequired,
    .parentType, .parentId
}) as properties
RETURN pid as parentId, properties
