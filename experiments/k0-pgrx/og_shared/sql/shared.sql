-- og_shared/sql/shared.sql — an extension_sql_file! that lives in the library crate.
-- Unqualified, so it lands in the installing extension's schema (@extschema@).
CREATE TABLE k0_shared_file_marker (note text);
INSERT INTO k0_shared_file_marker VALUES ('from og_shared/sql/shared.sql, after ' || (SELECT note FROM k0_edition_boot));
COMMENT ON TABLE k0_shared_file_marker IS 'schema=@extschema@';
