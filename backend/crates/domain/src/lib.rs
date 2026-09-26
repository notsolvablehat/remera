// Pure business logic for backend.
//
// Nothing in this crate should depend on axum, sqlx, tokio's networking
// types, or any other infra concern. `api` and `storage` depend
// on this crate; this crate depends on nothing infra-related.

pub mod container;
pub mod errors;
pub mod media;

pub use container::{Container, MAX_OWNED_CONTAINERS, Role, role_meets_minimum};
pub use errors::DomainError;
pub use media::{
    DEFAULT_MEDIA_LIMIT, DEFAULT_STORAGE_LIMIT_BYTES, MAX_UPLOAD_SIZE_BYTES, Media, MediaStatus,
};
