use std::fmt;

use tcc_ir::IrError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    Ir(IrError),
    ArtifactMismatch {
        expected: String,
        found: String,
    },
    FormatMismatch {
        found: u32,
        supported: u32,
    },
    MissingHostProtocolVersion,
    UnsupportedHostProtocol {
        got: u32,
        supported: u32,
    },
    LanguageSemanticsMismatch {
        expected: String,
        found: String,
    },
    InvalidContinuation(String),
    UnexpectedHostResponse {
        expected: &'static str,
        got: &'static str,
    },
    StackUnderflow,
    TypeError(String),
    UnknownInstruction {
        func: u32,
        pc: u32,
    },
    NoFrame,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Ir(err) => write!(f, "{err}"),
            CoreError::ArtifactMismatch { expected, found } => write!(
                f,
                "continuation is pinned to artifact `{expected}`, got `{found}`"
            ),
            CoreError::FormatMismatch { found, supported } => write!(
                f,
                "continuation format version {found} is incompatible with engine format {supported}"
            ),
            CoreError::MissingHostProtocolVersion => {
                write!(f, "host_protocol_version is required")
            }
            CoreError::UnsupportedHostProtocol { got, supported } => write!(
                f,
                "unsupported host_protocol_version {got} (engine supports {supported})"
            ),
            CoreError::LanguageSemanticsMismatch { expected, found } => write!(
                f,
                "continuation language_semantics_version `{found}` does not match artifact `{expected}`"
            ),
            CoreError::InvalidContinuation(message) => write!(f, "{message}"),
            CoreError::UnexpectedHostResponse { expected, got } => write!(
                f,
                "host response `{got}` does not match outstanding request `{expected}`"
            ),
            CoreError::StackUnderflow => write!(f, "operand stack underflow"),
            CoreError::TypeError(message) => write!(f, "{message}"),
            CoreError::UnknownInstruction { func, pc } => {
                write!(f, "unknown instruction in function {func} at pc {pc}")
            }
            CoreError::NoFrame => write!(f, "no active frame"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<IrError> for CoreError {
    fn from(value: IrError) -> Self {
        CoreError::Ir(value)
    }
}
