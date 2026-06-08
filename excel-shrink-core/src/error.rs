use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShrinkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Regex compilation error: {0}")]
    Regex(#[from] regex::Error),

    #[error("Invalid root node found in XML")]
    InvalidRootNode,

    #[error("Unexpected EOF while parsing sheet state")]
    UnexpectedEof,

    #[error("CDATA sections are not supported")]
    UnsupportedCData,

    #[error("Missing attribute: {0}")]
    MissingAttribute(String),
}

pub type Result<T> = std::result::Result<T, ShrinkError>;
