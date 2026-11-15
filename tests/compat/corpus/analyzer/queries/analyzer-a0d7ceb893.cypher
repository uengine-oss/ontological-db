UNWIND $items AS item
MATCH (t:TABLE {id: item.id, graph_owner: 'analyzer'})
SET t.analyzed_description = item.analyzed_description,
    t.logical_name = CASE WHEN coalesce(item.logical_name, '') <> ''
                          THEN item.logical_name ELSE t.logical_name END,
    t.description_source = CASE WHEN coalesce(t.description, '') = ''
                                THEN 'code' ELSE t.description_source END
