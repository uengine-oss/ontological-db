MATCH (gwt:GWT {parentType: $parent_type, parentId: $parent_id})
RETURN gwt {.id, .parentType, .parentId, .givenRef, .whenRef, .thenRef, .testCases} AS gwt
