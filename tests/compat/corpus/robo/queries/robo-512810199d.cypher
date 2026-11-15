MATCH (m:StoryboardPageMapping {commandId: $cid})
SET m.figmaPageName = $new_name,
    m.lastRenameAt = $now
