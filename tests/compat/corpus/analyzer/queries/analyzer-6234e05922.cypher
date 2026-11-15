UNWIND $items AS item
MATCH (c:COLUMN {id: item.id, graph_owner: 'analyzer'})
SET c.analyzed_description = item.analyzed_description,
    c.logical_name = CASE WHEN coalesce(item.logical_name, '') <> ''
                          THEN item.logical_name ELSE c.logical_name END,
    c.description_source = CASE WHEN coalesce(c.description, '') = ''
                                THEN 'code' ELSE c.description_source END
