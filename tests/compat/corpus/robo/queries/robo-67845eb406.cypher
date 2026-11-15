MATCH (c:FigmaComponent {bindingFileKey: $fk})
RETURN c
ORDER BY c.pageName, c.name
