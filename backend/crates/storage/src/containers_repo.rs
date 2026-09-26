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
        "SELECT COUNT(*) FROM container WHERE owner_id = $1",
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
        "SELECT id, owner_id, name, is_public, is_locked FROM container WHERE id = $1",
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
         WHERE m.user_id = $1
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
