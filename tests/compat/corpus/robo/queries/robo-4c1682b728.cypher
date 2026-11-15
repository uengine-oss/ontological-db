UNWIND $names AS n
OPTIONAL MATCH (evt1:Event {name: n})
OPTIONAL MATCH (evt2:Event {displayName: n})
OPTIONAL MATCH (evt3:Event)
  WHERE toLower(evt3.name) = toLower(n)
     OR toLower(evt3.displayName) = toLower(n)
WITH n, coalesce(evt1, evt2, evt3) AS evt
WHERE evt IS NOT NULL
RETURN n AS lookup, evt.id AS id LIMIT 1000
