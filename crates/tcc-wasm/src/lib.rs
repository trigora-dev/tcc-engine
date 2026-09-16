//! WASM exports for the TCC core.
//!
//! The WASM target has no WASI networking, filesystem, or threads. Hosts supply
//! those capabilities through the execution protocol in `tcc-core`.

pub use tcc_core::{
    ChildSpec, EffectRecord, EffectStatus, Engine, EngineOutcome, HostRequest, HostResponse,
    WaitRegistration,
};
pub use tcc_ir::{validate, Artifact, EngineCaps, ENGINE_FORMAT_VERSION};

/// Stable numeric export for host bindings that load the module.
#[no_mangle]
pub extern "C" fn tcc_engine_format_version() -> u32 {
    ENGINE_FORMAT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_current_format_version() {
        assert_eq!(tcc_engine_format_version(), ENGINE_FORMAT_VERSION);
        assert_eq!(ENGINE_FORMAT_VERSION, 1);
    }
}
