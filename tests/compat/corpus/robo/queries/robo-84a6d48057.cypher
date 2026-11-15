MERGE (d:DocumentPassage {id: $id, session_id: $sid}) SET d.heading = $heading, d.text = $text, d.page = $page,     d.char_start = $cs, d.char_end = $ce, d.chunk_method = $method
