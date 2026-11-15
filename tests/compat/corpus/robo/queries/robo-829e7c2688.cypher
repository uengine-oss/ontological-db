MERGE (us:UserStory {id: $id})
ON CREATE SET us.name = $name,
              us.role = $role,
              us.action = $action,
              us.benefit = $benefit,
              us.priority = $priority,
              us.status = $status,
              us.uiDescription = $ui_description,
              us.displayName = $display_name,
              us.sourceScreenName = $source_screen_name,
              us.sourceUnitId = $source_unit_id,
              us.sequence = $sequence,
              us.acceptanceCriteria = CASE WHEN $acceptance_criteria IS NOT NULL THEN $acceptance_criteria ELSE [] END,
              us.createdAt = datetime()
ON MATCH SET us.name = CASE WHEN us.name IS NULL THEN $name ELSE us.name END,
             us.role = CASE WHEN $role IS NOT NULL AND $role <> '' THEN $role ELSE us.role END,
             us.action = CASE WHEN $action IS NOT NULL AND $action <> '' THEN $action ELSE us.action END,
             us.benefit = CASE WHEN $benefit IS NOT NULL AND $benefit <> '' THEN $benefit ELSE us.benefit END,
             us.priority = CASE WHEN $priority IS NOT NULL AND $priority <> '' THEN $priority ELSE us.priority END,
             us.status = CASE WHEN $status IS NOT NULL AND $status <> '' THEN $status ELSE us.status END,
             us.uiDescription = CASE WHEN $ui_description IS NOT NULL AND $ui_description <> '' THEN $ui_description ELSE us.uiDescription END,
             us.displayName = CASE WHEN $display_name IS NOT NULL AND $display_name <> '' THEN $display_name ELSE us.displayName END,
             us.sourceScreenName = CASE WHEN $source_screen_name IS NOT NULL AND $source_screen_name <> '' THEN $source_screen_name ELSE us.sourceScreenName END,
             us.sourceUnitId = CASE WHEN $source_unit_id IS NOT NULL AND $source_unit_id <> '' THEN $source_unit_id ELSE us.sourceUnitId END,
             us.sequence = CASE WHEN $sequence IS NOT NULL THEN $sequence ELSE us.sequence END,
             us.acceptanceCriteria = CASE
                 WHEN $acceptance_criteria IS NOT NULL AND NOT coalesce(us.criteriaUserEdited, false) THEN $acceptance_criteria
                 ELSE us.acceptanceCriteria
             END,
             us.updatedAt = datetime()
RETURN us {.id, .name, .role, .action, .benefit, .priority, .status, .sequence,
           uiDescription: us.uiDescription, displayName: us.displayName,
           sourceScreenName: us.sourceScreenName, sourceUnitId: us.sourceUnitId,
           acceptanceCriteria: coalesce(us.acceptanceCriteria, []),
           criteriaUserEdited: coalesce(us.criteriaUserEdited, false),
           criteriaEditedAt: us.criteriaEditedAt} as user_story
