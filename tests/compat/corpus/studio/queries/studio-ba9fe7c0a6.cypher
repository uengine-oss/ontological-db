MATCH (t:Table)
WHERE (t.datasource = $datasource OR t.db = $datasource)
  AND t.schema = $schema
  AND ($tables = [] OR t.name IN $tables)
OPTIONAL MATCH (t)-[:HAS_COLUMN]->(c:Column)
WITH t, c ORDER BY coalesce(c.ordinal_position, 0), c.name
RETURN t.name AS table_name,
       t.description AS table_description,
       collect({
         name: c.name,
         sql_type: coalesce(c.type, c.dtype, ''),
         nullable: coalesce(c.nullable, true),
         primary_key: coalesce(c.primary_key, c.is_primary_key, false),
         description: coalesce(c.description, '')
       }) AS columns
ORDER BY t.name
