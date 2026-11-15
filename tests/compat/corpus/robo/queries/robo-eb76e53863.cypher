MATCH (n)
WHERE NOT (n:FigmaBinding OR n:StoryboardPageMapping OR n:BindingHistoryEvent OR n:FigmaComponent)
WITH labels(n)[0] as label, count(n) as count
RETURN collect({label: label, count: count}) as counts
