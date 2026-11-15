MERGE (b:FigmaBinding {id: $id})
SET b.figmaFileKey = $file_key,
    b.figmaFileName = $file_name,
    b.connectedBy = $connected_by,
    b.connectedAt = $now,
    b.lastSyncAt = null,
    b.status = 'active'
RETURN b
