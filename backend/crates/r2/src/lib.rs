// Thin wrapper around aws-sdk-s3, pointed at Cloudflare R2 (S3-compatible).
// Presigned URLs only — this crate never reads/writes object bytes itself,
// so the api server never sits in the upload/download data path.

pub mod client;
pub mod keys;
pub mod presign;

pub use aws_sdk_s3::Client;
pub use client::build_client;
pub use presign::{
    R2Error, delete_object, presigned_get_url, presigned_put_url, verify_uploaded_object,
};
