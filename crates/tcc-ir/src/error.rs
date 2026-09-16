use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrError {
    UnsupportedFormat { found: u32, supported: u32 },
    UnsupportedEngineFeature(String),
    UnsupportedHostCapability(String),
    EmptyProgram,
    MissingEntry(u32),
    DuplicateFunction(u32),
    JumpOutOfRange { func: u32, target: u32, len: usize },
    SpanLengthMismatch { func: u32, index: usize },
    EmptyArtifactHash,
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrError::UnsupportedFormat { found, supported } => write!(
                f,
                "unsupported engine format version {found} (engine supports {supported})"
            ),
            IrError::UnsupportedEngineFeature(id) => {
                write!(f, "required engine feature `{id}` is not supported")
            }
            IrError::UnsupportedHostCapability(id) => {
                write!(f, "required host capability `{id}` is not supported")
            }
            IrError::EmptyProgram => write!(f, "artifact has no functions"),
            IrError::MissingEntry(id) => write!(f, "entry function {id} is missing"),
            IrError::DuplicateFunction(id) => write!(f, "duplicate function id {id}"),
            IrError::JumpOutOfRange { func, target, len } => write!(
                f,
                "jump target {target} is out of range in function {func} ({len} instructions)"
            ),
            IrError::SpanLengthMismatch { func, index } => write!(
                f,
                "instruction {index} in function {func} has no matching source span"
            ),
            IrError::EmptyArtifactHash => write!(f, "artifact hash must not be empty"),
        }
    }
}

impl std::error::Error for IrError {}
