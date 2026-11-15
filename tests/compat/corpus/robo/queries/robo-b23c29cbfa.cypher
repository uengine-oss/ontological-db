UNWIND $fn_names AS fn
OPTIONAL MATCH (f)
  WHERE f.name = fn AND (f:FUNCTION OR f:PROCEDURE OR f:METHOD OR f:TRIGGER)
WITH fn, collect(f.summary) AS summaries
RETURN fn,
       head([s IN summaries WHERE s IS NOT NULL AND s <> '']) AS summary
