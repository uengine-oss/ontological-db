MATCH (n:Command {id: $id}) SET n.sequence = $seq, n.updatedAt = datetime()
