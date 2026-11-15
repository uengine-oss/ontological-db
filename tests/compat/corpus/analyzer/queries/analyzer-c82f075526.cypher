MATCH (m:EMBED_META {graph_owner: $graph_owner}) RETURN m.fingerprint AS fp, m.embedded_count AS count
