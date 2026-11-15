MERGE (m:ANALYZER_OWNED:EMBED_META {graph_owner: 'analyzer'}) SET m.fingerprint = $fp, m.dim = $dim, m.embedded_count = $count
