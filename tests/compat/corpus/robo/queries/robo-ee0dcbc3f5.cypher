UNWIND $rows AS r
MERGE (us:UserStory {id: r.id})
  ON CREATE SET us.name = r.name,
                us.role = r.role,
                us.action = r.action,
                us.benefit = r.benefit,
                us.priority = r.priority,
                us.status = r.status,
                us.uiDescription = r.ui_description,
                us.displayName = r.display_name,
                us.sourceScreenName = r.source_screen_name,
                us.sourceUnitId = r.source_unit_id,
                us.sequence = r.sequence,
                us.acceptanceCriteria = r.acceptance_criteria,
                us.createdAt = datetime()
  ON MATCH SET us.name = CASE WHEN us.name IS NULL THEN r.name ELSE us.name END,
               us.role = CASE WHEN r.role IS NOT NULL AND r.role <> '' THEN r.role ELSE us.role END,
               us.action = CASE WHEN r.action IS NOT NULL AND r.action <> '' THEN r.action ELSE us.action END,
               us.benefit = CASE WHEN r.benefit IS NOT NULL AND r.benefit <> '' THEN r.benefit ELSE us.benefit END,
               us.priority = CASE WHEN r.priority IS NOT NULL AND r.priority <> '' THEN r.priority ELSE us.priority END,
               us.status = CASE WHEN r.status IS NOT NULL AND r.status <> '' THEN r.status ELSE us.status END,
               us.uiDescription = CASE WHEN r.ui_description IS NOT NULL AND r.ui_description <> '' THEN r.ui_description ELSE us.uiDescription END,
               us.displayName = CASE WHEN r.display_name IS NOT NULL AND r.display_name <> '' THEN r.display_name ELSE us.displayName END,
               us.sourceScreenName = CASE WHEN r.source_screen_name IS NOT NULL AND r.source_screen_name <> '' THEN r.source_screen_name ELSE us.sourceScreenName END,
               us.sourceUnitId = CASE WHEN r.source_unit_id IS NOT NULL AND r.source_unit_id <> '' THEN r.source_unit_id ELSE us.sourceUnitId END,
               us.sequence = CASE WHEN r.sequence IS NOT NULL THEN r.sequence ELSE us.sequence END,
               us.acceptanceCriteria = CASE
                   WHEN coalesce(us.criteriaUserEdited, false) THEN us.acceptanceCriteria
                   WHEN r.acceptance_criteria IS NOT NULL AND size(r.acceptance_criteria) > 0 THEN r.acceptance_criteria
                   ELSE us.acceptanceCriteria
               END,
               us.updatedAt = datetime()
RETURN us {.id, .name, .role, .action, .benefit, .priority, .status, .sequence,
           uiDescription: us.uiDescription, displayName: us.displayName,
           sourceScreenName: us.sourceScreenName, sourceUnitId: us.sourceUnitId,
           acceptanceCriteria: us.acceptanceCriteria,
           criteriaUserEdited: coalesce(us.criteriaUserEdited, false),
           criteriaEditedAt: us.criteriaEditedAt} AS result
