use thiserror::Error;

#[derive(Debug, Error)]
pub enum DdcError {
    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("monitor not found: {0}")]
    NotFound(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("DDC/CI error: {0}")]
    DdcFailed(String),

    #[error("backlight error: {0}")]
    BacklightFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
