MATCH (f {_owner: 'analyzer'})
WHERE f._id = $fid OR f.name = $fid
RETURN f._id AS id, f.name AS name, f.summary AS summary,
       f.start_line AS start_line, f.end_line AS end_line,
       f.file_path AS file_path,
       f.code_text AS code_text
LIMIT 1
