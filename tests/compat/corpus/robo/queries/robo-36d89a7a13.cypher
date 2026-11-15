MATCH (b:FigmaBinding {id: $id})
WHERE b.status <> 'disconnected'
RETURN b
