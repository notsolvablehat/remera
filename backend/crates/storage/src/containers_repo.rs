use domain::{Container, DomainError, MAX_OWNED_CONTAINERS, Role};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::members_repo;

#[derive(Debug, Error)]
pub enum ContainersRepoError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub async fn create_with_owner(
    pool: &PgPool,
    owner_id: &str,
    name: &str,
) -> Result<Container, ContainersRepoError> {
    let mut tx = pool.begin().await?;

    // Row lock isn't needed here beyond the transaction itself — two
    // concurrent creates from the *same* owner racing past this count
    // is an acceptable, narrow edge case for a soft quota like this one.
    let owned: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM container WHERE owner_id = $1 AND deleted_at IS NULL",
        owner_id
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);

    if owned >= MAX_OWNED_CONTAINERS {
        return Err(DomainError::QuotaExceeded.into());
    }

    let row = sqlx::query!(
        "INSERT INTO container (owner_id, name) VALUES ($1, $2)
         RETURNING id, owner_id, name, is_public, is_locked",
        owner_id,
        name
    )
    .fetch_one(&mut *tx)
    .await?;

    members_repo::insert_owner_member(&mut *tx, row.id, owner_id).await?;

    tx.commit().await?;

    Ok(Container {
        id: row.id,
        owner_id: row.owner_id,
        name: row.name,
        is_public: row.is_public,
        is_locked: row.is_locked,
    })
}

pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Container>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT id, owner_id, name, is_public, is_locked
         FROM container WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Container {
        id: r.id,
        owner_id: r.owner_id,
        name: r.name,
        is_public: r.is_public,
        is_locked: r.is_locked,
    }))
}

pub struct ContainerWithRole {
    pub container: Container,
    pub role: Role,
}

pub async fn list_for_user(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<ContainerWithRole>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT c.id, c.owner_id, c.name, c.is_public, c.is_locked, m.role
         FROM container c
         JOIN container_member m ON m.container_id = c.id
         WHERE m.user_id = $1 AND c.deleted_at IS NULL
         ORDER BY c.created_at DESC",
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let role = r.role.parse().ok()?;
            Some(ContainerWithRole {
                container: Container {
                    id: r.id,
                    owner_id: r.owner_id,
                    name: r.name,
                    is_public: r.is_public,
                    is_locked: r.is_locked,
                },
                role,
            })
        })
        .collect())
}

/// Updates name/storage_limit/media_limit. `None` leaves a field
/// unchanged. Caller (the route handler) is responsible for requiring at
/// least one field to be set — this will happily run a no-op update
/// otherwise.
pub async fn update_metadata(
    pool: &PgPool,
    id: Uuid,
    name: Option<&str>,
    storage_limit_bytes: Option<i64>,
    media_limit: Option<i32>,
) -> Result<Option<Container>, sqlx::Error> {
    let row = sqlx::query!(
        "UPDATE container
         SET name = COALESCE($2, name),
             storage_limit = COALESCE($3, storage_limit),
             media_limit = COALESCE($4, media_limit)
         WHERE id = $1 AND deleted_at IS NULL
         RETURNING id, owner_id, name, is_public, is_locked",
        id,
        name,
        storage_limit_bytes,
        media_limit
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Container {
        id: r.id,
        owner_id: r.owner_id,
        name: r.name,
        is_public: r.is_public,
        is_locked: r.is_locked,
    }))
}

pub async fn set_locked(pool: &PgPool, id: Uuid, locked: bool) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE container SET is_locked = $2 WHERE id = $1 AND deleted_at IS NULL",
        id,
        locked
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Soft-delete only — sets `deleted_at`, doesn't touch R2 objects or
/// remove rows. Purging is a background-job concern, not implemented yet.
pub async fn soft_delete(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE container SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL",
        id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub struct ContainerUsage {
    pub storage_bytes: i64,
    pub storage_limit: i64,
    pub media_count: i32,
    pub media_limit: i32,
    pub member_count: i64,
}

pub async fn get_usage(pool: &PgPool, id: Uuid) -> Result<Option<ContainerUsage>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT c.storage_bytes, c.storage_limit, c.media_count, c.media_limit,
                (SELECT COUNT(*) FROM container_member cm WHERE cm.container_id = c.id) AS member_count
         FROM container c
         WHERE c.id = $1 AND c.deleted_at IS NULL",
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ContainerUsage {
        storage_bytes: r.storage_bytes,
        storage_limit: r.storage_limit,
        media_count: r.media_count,
        media_limit: r.media_limit,
        member_count: r.member_count.unwrap_or(0),
    }))
}

/// Lazily creates the container's share-link id if it doesn't have one
/// yet, otherwise returns the existing one — so `GET .../share-link` is
/// idempotent and doesn't invalidate previously distributed links just
/// by being called again.
pub async fn get_or_create_share_link_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE container
         SET share_link_id = COALESCE(share_link_id, gen_random_uuid())
         WHERE id = $1 AND deleted_at IS NULL
         RETURNING share_link_id",
        id
    )
    .fetch_optional(pool)
    .await
    .map(|opt| opt.flatten())
}

/// Assigns a brand-new share-link id, which invalidates every
/// previously issued token for this container — a decrypted token's
/// embedded id no longer matches what's stored, so it fails validation
/// even though the ciphertext itself still decrypts fine.
pub async fn rotate_share_link_id(pool: &PgPool, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE container SET share_link_id = gen_random_uuid()
         WHERE id = $1 AND deleted_at IS NULL
         RETURNING share_link_id",
        id
    )
    .fetch_optional(pool)
    .await
    .map(|opt| opt.flatten())
}

pub struct SharePreview {
    pub name: String,
    pub is_locked: bool,
    pub media_count: i64,
}

/// Resolves a decrypted share token's `(container_id, share_link_id)`
/// pair to a lightweight preview — only succeeds if `share_link_id`
/// matches what's currently stored (i.e. hasn't been rotated since the
/// token was issued).
pub async fn get_share_preview(
    pool: &PgPool,
    container_id: Uuid,
    share_link_id: Uuid,
) -> Result<Option<SharePreview>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT c.name, c.is_locked,
                (SELECT COUNT(*) FROM media m WHERE m.container_id = c.id AND m.status = 'ready') AS media_count
         FROM container c
         WHERE c.id = $1 AND c.share_link_id = $2 AND c.deleted_at IS NULL",
        container_id,
        share_link_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| SharePreview {
        name: r.name,
        is_locked: r.is_locked,
        media_count: r.media_count.unwrap_or(0),
    }))
}
