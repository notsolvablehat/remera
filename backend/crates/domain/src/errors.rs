use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("not found")]
    NotFound,

    #[error("quota exceeded")]
    QuotaExceeded,

    #[error("container is locked")]
    LockedContainer,

    #[error("upload not found in object storage yet")]
    UploadNotVerified,

    #[error("the owner must transfer ownership before leaving or being removed")]
    OwnerTransferRequired,
}
