MATCH (n)
WHERE NOT (n:FigmaBinding OR n:StoryboardPageMapping OR n:BindingHistoryEvent OR n:FigmaComponent)
DETACH DELETE n
