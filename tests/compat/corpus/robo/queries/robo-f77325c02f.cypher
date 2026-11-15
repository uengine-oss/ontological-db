MATCH (c:FigmaComponent {bindingFileKey: $fk}) RETURN count(c) AS n
