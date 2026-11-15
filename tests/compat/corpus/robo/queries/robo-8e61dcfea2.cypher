MATCH (m:StoryboardPageMapping {commandId: $cid}) SET m.status = 'archived'
