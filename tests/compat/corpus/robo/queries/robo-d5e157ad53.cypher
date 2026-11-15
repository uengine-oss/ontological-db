MATCH (f {_owner: 'analyzer'}) WHERE (f:FUNCTION OR f:PROCEDURE OR f:METHOD OR f:TRIGGER)   AND f.name IS NOT NULL AND f.name <> '' RETURN f.name AS fn, f.summary AS summary
