MERGE (evt:Event {id: $evt_id})
SET evt.name = $name,
    evt.description = $description,
    evt.version = 1,
    evt.createdAt = datetime()
RETURN evt.id as id
