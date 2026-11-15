MATCH (c:FigmaComponent {bindingFileKey: $fk})
WITH c, count(c) AS n
DETACH DELETE c
RETURN n
