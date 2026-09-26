// Pure business logic for backend.
//
// Nothing in this crate should depend on axum, sqlx, tokio's networking
// types, or any other infra concern. `api` and `storage` depend
// on this crate; this crate depends on nothing infra-related.

pub mod container;
pub mod errors;

pub use container::{Container, MAX_OWNED_CONTAINERS, Role, role_meets_minimum};
pub use errors::DomainError;
