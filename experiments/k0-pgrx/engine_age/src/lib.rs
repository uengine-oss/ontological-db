//! A shell around the engine compiled as a library. Naming the crate is what links it (and with
//! it every #[pg_extern], extension_sql_file! and pg_module_magic! it holds).
extern crate og_engine_lib as _;
