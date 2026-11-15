MATCH (c:FigmaComponent {bindingFileKey: $fk})
WHERE NOT c.figmaNodeId IN $kept
WITH c, count(c) AS n
DETACH DELETE c
RETURN n
