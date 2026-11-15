MATCH (u:UI {id: $uid})
SET u.figmaSyncStatus = 'ok',
    u.figmaSyncLastError = null,
    u.figmaSyncLastAttemptAt = datetime(),
    u.figmaPageId = $page_id,
    u.figmaNodeId = $node_id
