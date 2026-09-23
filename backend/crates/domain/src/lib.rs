// Pure business logic for backend.
//
// Nothing in this crate should depend on axum, sqlx, tokio's networking
// types, or any other infra concern. `api` and `storage` depend
// on this crate; this crate depends on nothing infra-related.

pub mod errors;

pub use errors::DomainError;
