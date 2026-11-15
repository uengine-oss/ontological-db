MATCH (evt:Event {name: $name}) RETURN evt.sequence AS seq LIMIT 1
