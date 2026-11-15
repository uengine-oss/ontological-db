MATCH (b:FigmaBinding {id: $id}) SET b.lastSyncAt = $now
