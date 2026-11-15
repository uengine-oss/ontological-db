MATCH (evt:Event)
WHERE evt.name = $event_name
   OR evt.displayName = $event_name
   OR toLower(evt.name) = toLower($event_name)
   OR toLower(evt.displayName) = toLower($event_name)
RETURN evt.id AS id, evt.name AS name, evt.displayName AS displayName
