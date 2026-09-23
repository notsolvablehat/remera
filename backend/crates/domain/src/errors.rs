use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("not found")]
    NotFound,

    #[error("quota exceeded")]
    QuotaExceeded,

    #[error("container is locked")]
    LockedContainer,
}
