use std::time::Duration;

use aws_sdk_s3::{
    Client,
    error::SdkError,
    operation::{
        delete_object::DeleteObjectError, get_object::GetObjectError, head_object::HeadObjectError,
        put_object::PutObjectError,
    },
    presigning::{PresigningConfig, PresigningConfigError},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum R2Error {
    #[error("invalid presigning duration: {0}")]
    PresigningConfig(#[from] PresigningConfigError),
    #[error("failed to presign upload url: {0}")]
    PresignPut(Box<SdkError<PutObjectError>>),
    #[error("failed to presign download url: {0}")]
    PresignGet(Box<SdkError<GetObjectError>>),
    #[error("failed to verify uploaded object: {0}")]
    Head(Box<SdkError<HeadObjectError>>),
    #[error("failed to delete object: {0}")]
    Delete(Box<SdkError<DeleteObjectError>>),
}

impl From<SdkError<PutObjectError>> for R2Error {
    fn from(err: SdkError<PutObjectError>) -> Self {
        R2Error::PresignPut(Box::new(err))
    }
}

impl From<SdkError<GetObjectError>> for R2Error {
    fn from(err: SdkError<GetObjectError>) -> Self {
        R2Error::PresignGet(Box::new(err))
    }
}

impl From<SdkError<HeadObjectError>> for R2Error {
    fn from(err: SdkError<HeadObjectError>) -> Self {
        R2Error::Head(Box::new(err))
    }
}

impl From<SdkError<DeleteObjectError>> for R2Error {
    fn from(err: SdkError<DeleteObjectError>) -> Self {
        R2Error::Delete(Box::new(err))
    }
}

/// Presigned PUT URL the client uploads bytes to directly — the server
/// never touches the file body. `content_type` is baked into the
/// signature, so the client's PUT request must send the exact same
/// `Content-Type` header or R2 will reject it.
pub async fn presigned_put_url(
    client: &Client,
    bucket: &str,
    key: &str,
    content_type: &str,
    expires_in: Duration,
) -> Result<String, R2Error> {
    let presigned = client
        .put_object()
        .bucket(bucket)
        .key(key)
        .content_type(content_type)
        .presigned(PresigningConfig::expires_in(expires_in)?)
        .await?;

    Ok(presigned.uri().to_string())
}

/// Presigned GET URL for downloading, with `Content-Disposition:
/// attachment; filename="..."` baked in so the browser saves the file
/// under its original name rather than the opaque object key.
pub async fn presigned_get_url(
    client: &Client,
    bucket: &str,
    key: &str,
    download_filename: &str,
    expires_in: Duration,
) -> Result<String, R2Error> {
    let presigned = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .response_content_disposition(format!("attachment; filename=\"{download_filename}\""))
        .presigned(PresigningConfig::expires_in(expires_in)?)
        .await?;

    Ok(presigned.uri().to_string())
}

/// HEAD the object to confirm the client's presigned PUT actually landed,
/// returning the size R2 recorded (compared against the declared size at
/// upload-creation time by the caller).
pub async fn verify_uploaded_object(
    client: &Client,
    bucket: &str,
    key: &str,
) -> Result<Option<i64>, R2Error> {
    let head = client.head_object().bucket(bucket).key(key).send().await;

    match head {
        Ok(output) => Ok(output.content_length()),
        Err(SdkError::ServiceError(e)) if e.err().is_not_found() => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Deletes an object outright — used both for aborting a pending upload
/// (client never finished the PUT) and for a confirmed delete of a ready
/// media item. Treats "already gone" as success, not an error.
pub async fn delete_object(client: &Client, bucket: &str, key: &str) -> Result<(), R2Error> {
    client
        .delete_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await?;
    Ok(())
}
