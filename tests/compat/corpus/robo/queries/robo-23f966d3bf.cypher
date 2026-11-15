MATCH (b:FigmaBinding {id: $bid})
MERGE (c:FigmaComponent {bindingFileKey: $fk, figmaNodeId: $nid})
ON CREATE SET c.id = $newid
SET c.name = $name,
    c.pageName = $page_name,
    c.widthPx = $w,
    c.heightPx = $h,
    c.vlmDescription = $desc,
    c.figmaKey = $fkey,
    c.figmaNodeLastModified = $last_mod,
    c.scannedAt = $now
MERGE (b)-[:HAS_COMPONENT]->(c)
RETURN c
