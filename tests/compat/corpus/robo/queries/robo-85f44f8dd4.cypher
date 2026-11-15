MATCH (inv:Invariant {id: $id}) SET inv.statement = $statement RETURN inv.id AS id
