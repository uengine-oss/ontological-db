//! Stand-in for `og-pg-common`: SQL-facing code written once, shipped by two extensions.
//!
//! Nothing here knows which extension it is linked into. The extension crate tells it in
//! `_PG_init` (`set_edition`). Each extension's shared object carries its own copy of this crate,
//! so `EDITION` (and every other static here) is per extension even inside one backend.

use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};
use pgrx::prelude::*;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static EDITION: OnceLock<&'static str> = OnceLock::new();

/// Called by the extension crate's `_PG_init` with its extension name.
pub fn set_edition(name: &'static str) {
    let _ = EDITION.set(name);
}

pub fn edition() -> &'static str {
    EDITION.get().copied().unwrap_or("<unset>")
}

/// File name of the shared object this code was loaded from.
pub fn so_name() -> String {
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }
    unsafe extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    }
    let mut info = DlInfo {
        dli_fname: std::ptr::null(),
        dli_fbase: std::ptr::null_mut(),
        dli_sname: std::ptr::null(),
        dli_saddr: std::ptr::null_mut(),
    };
    let rc = unsafe { dladdr(so_name as *const c_void, &mut info) };
    if rc == 0 || info.dli_fname.is_null() {
        return "<dladdr failed>".into();
    }
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned();
    path.rsplit('/').next().unwrap_or(&path).to_string()
}

// (1) A plain pg_extern, no schema: lands in the installing extension's schema (@extschema@).
#[pg_extern]
fn k0_hello() -> String {
    format!("og_shared::k0_hello in {} ({})", edition(), so_name())
}

// (2) Which shared object answers.
#[pg_extern]
fn k0_whoami() -> String {
    format!("{} ({})", edition(), so_name())
}

// (3) A #[pg_schema] module: the schema name is fixed when the library is compiled.
#[cfg(feature = "fixed-schema")]
#[pg_schema]
mod k0_fixed {
    use pgrx::prelude::*;

    #[pg_extern]
    fn k0_in_fixed_schema() -> String {
        super::k0_whoami()
    }
}

// (4) extension_sql! and extension_sql_file! in the library, each requiring an entity that the
//     *extension* crate defines (by name) and, for the first, a function of this crate (by path).
extension_sql!(
    r#"
CREATE TABLE k0_shared_marker (note text);
INSERT INTO k0_shared_marker VALUES ('from og_shared extension_sql!, after ' || (SELECT note FROM k0_edition_boot));
"#,
    name = "k0_shared_sql",
    requires = ["k0_edition_boot", k0_hello]
);
extension_sql_file!("../sql/shared.sql", name = "k0_shared_file", requires = ["k0_edition_boot"]);

// (5) A GUC defined the first time it is needed, the way engine/src/fabric/mod.rs:370 defines
//     `ontological.fabric_inline_passwords`. Both shared objects carry this code and this name.
static PROBE: GucSetting<i32> = GucSetting::<i32>::new(7);
static PROBE_DEFINED: AtomicBool = AtomicBool::new(false);

#[pg_extern]
fn k0_guc_probe() -> i32 {
    if !PROBE_DEFINED.swap(true, Ordering::Relaxed) {
        GucRegistry::define_int_guc(
            c"k0.shared_probe",
            c"K0 probe",
            c"Defined by og_shared on first call.",
            &PROBE,
            0,
            100,
            GucContext::Userset,
            GucFlags::default(),
        );
    }
    PROBE.get()
}

// (6) SPI with an unqualified function name — what compiled Cypher does with og_node_json(...),
//     og_vlp(...), ...: resolved through search_path, not through "the extension I belong to".
#[pg_extern]
fn k0_spi_unqualified() -> String {
    let got = Spi::get_one::<String>("SELECT k0_whoami()").ok().flatten();
    format!("spi-unqualified -> {}", got.unwrap_or_else(|| "<null>".into()))
}

// (7) The same call qualified with this extension's own schema, looked up by edition name.
#[pg_extern]
fn k0_spi_qualified() -> String {
    let schema = Spi::get_one::<String>(&format!(
        "SELECT n.nspname::text FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace \
         WHERE e.extname = '{}'",
        edition()
    ))
    .ok()
    .flatten()
    .unwrap_or_else(|| "<no schema>".into());
    let got = Spi::get_one::<String>(&format!("SELECT \"{schema}\".k0_whoami()")).ok().flatten();
    format!("spi-qualified -> {}", got.unwrap_or_else(|| "<null>".into()))
}
