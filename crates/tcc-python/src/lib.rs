//! Native Python embedding of the TCC core.
//!
//! JSON crosses the boundary the same way as the WASM C ABI. This crate does
//! not compile source and does not persist checkpoints.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use tcc_core::{decode_response, encode_outcome, Engine};
use tcc_ir::{decode_artifact, EngineCaps};
use tcc_state::{decode_continuation, encode_continuation};

#[pyclass]
struct EngineBinding {
    inner: Engine,
}

#[pymethods]
impl EngineBinding {
    #[new]
    fn new(artifact_json: &str, execution_id: &str) -> PyResult<Self> {
        let artifact = decode_artifact(artifact_json).map_err(py_err)?;
        let inner =
            Engine::start(artifact, execution_id, &EngineCaps::current()).map_err(py_err)?;
        Ok(Self { inner })
    }

    #[staticmethod]
    fn resume(artifact_json: &str, continuation_json: &str) -> PyResult<Self> {
        let artifact = decode_artifact(artifact_json).map_err(py_err)?;
        let continuation = decode_continuation(continuation_json.as_bytes()).map_err(py_err)?;
        let inner =
            Engine::resume(artifact, continuation, &EngineCaps::current()).map_err(py_err)?;
        Ok(Self { inner })
    }

    fn run_until_host(&mut self, budget: u32) -> PyResult<String> {
        encode_outcome(&self.inner.run_until_host(budget)).map_err(py_err)
    }

    fn apply_response(&mut self, json: &str) -> PyResult<()> {
        let response = decode_response(json).map_err(py_err)?;
        self.inner.apply_host_response(response).map_err(py_err)
    }

    fn continuation_json(&self) -> PyResult<String> {
        let bytes = encode_continuation(self.inner.continuation()).map_err(py_err)?;
        String::from_utf8(bytes).map_err(|err| PyRuntimeError::new_err(err.to_string()))
    }
}

fn py_err<E: std::fmt::Display>(err: E) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

#[pymodule]
fn tcc_engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<EngineBinding>()?;
    Ok(())
}
