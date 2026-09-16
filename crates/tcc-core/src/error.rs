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
    Terminal(&'static str),
    UnexpectedHostResponse {
        expected: &'static str,
        got: &'static str,
    },
    StackUnderflow,
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
            CoreError::Terminal(status) => write!(f, "execution has already {status}"),
            CoreError::UnexpectedHostResponse { expected, got } => write!(
                f,
                "host response `{got}` does not match outstanding request `{expected}`"
            ),
            CoreError::StackUnderflow => write!(f, "operand stack underflow"),
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
