// graph: *
// The fabric datasource every client's fabric statements read (one per database, so it is made once).
// {{PGHOST}} {{PGPORT}} {{PGDATABASE}} {{PGUSER}} are filled by run.py with the target's connection.
CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {host: '{{PGHOST}}', port: {{PGPORT}}, database: '{{PGDATABASE}}', schema: 'compat_erp', user: '{{PGUSER}}', password: ''};

// a second one, for the DROP DATASOURCE statements
CREATE DATASOURCE erp_old WITH ENGINE = 'postgres', PARAMETERS = {host: '{{PGHOST}}', port: {{PGPORT}}, database: '{{PGDATABASE}}', schema: 'compat_erp', user: '{{PGUSER}}', password: ''};
