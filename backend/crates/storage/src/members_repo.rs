use chrono::{DateTime, Utc};
use domain::{DomainError, Role};
use sqlx::{PgExecutor, PgPool};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum MembersRepoError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

pub struct MemberRecord {
    pub user_id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub role: Role,
    pub joined_at: DateTime<Utc>,
}

pub async fn insert_owner_member(
    executor: impl PgExecutor<'_>,
    container_id: Uuid,
    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO container_member (container_id, user_id, role) VALUES ($1, $2, 'owner')",
        container_id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_role(
    executor: impl PgExecutor<'_>,
    container_id: Uuid,
    user_id: &str,
) -> Result<Option<Role>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT role FROM container_member WHERE container_id = $1 AND user_id = $2",
        container_id,
        user_id
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.and_then(|r| r.role.parse().ok()))
}

pub async fn list_for_container(
    pool: &PgPool,
    container_id: Uuid,
) -> Result<Vec<MemberRecord>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT cm.user_id, u.name, u.email, cm.role, cm.created_at
         FROM container_member cm
         JOIN users u ON u.id = cm.user_id
         WHERE cm.container_id = $1
         ORDER BY cm.created_at ASC",
        container_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let role = r.role.parse().ok()?;
            Some(MemberRecord {
                user_id: r.user_id,
                name: r.name,
                email: r.email,
                role,
                joined_at: r.created_at,
            })
        })
        .collect())
}

/// Direct role change — Editor/Viewer only. Changing to/from Owner goes
/// through `transfer_ownership` instead, which atomically keeps "exactly
/// one owner" true; the CHECK constraint on `container_member.role`
/// would reject `'owner'` here anyway, but the real gate is the
/// caller-side validation in `routes/members.rs`, which stops the
/// request before it gets here.
pub async fn update_role(
    pool: &PgPool,
    container_id: Uuid,
    user_id: &str,
    role: Role,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE container_member SET role = $3 WHERE container_id = $1 AND user_id = $2",
        container_id,
        user_id,
        role.as_str()
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Removes a member (owner removing someone else, or a member leaving).
/// Refuses to remove the container's current owner — they have to
/// `transfer_ownership` first, otherwise the container would end up
/// with zero owners.
pub async fn remove_member(
    pool: &PgPool,
    container_id: Uuid,
    user_id: &str,
) -> Result<bool, MembersRepoError> {
    let owner_id: Option<String> = sqlx::query_scalar!(
        "SELECT owner_id FROM container WHERE id = $1 AND deleted_at IS NULL",
        container_id
    )
    .fetch_optional(pool)
    .await?;

    if owner_id.as_deref() == Some(user_id) {
        return Err(DomainError::OwnerTransferRequired.into());
    }

    let result = sqlx::query!(
        "DELETE FROM container_member WHERE container_id = $1 AND user_id = $2",
        container_id,
        user_id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Atomically swaps the owner role between the current owner and
/// `new_owner_id`: `new_owner_id` becomes Owner, the previous owner
/// takes whatever role `new_owner_id` held before (a real swap, not a
/// flat demotion to Editor).
pub async fn transfer_ownership(
    pool: &PgPool,
    container_id: Uuid,
    current_owner_id: &str,
    new_owner_id: &str,
) -> Result<(), MembersRepoError> {
    let mut tx = pool.begin().await?;

    let new_owner_role: Option<String> = sqlx::query_scalar!(
        "SELECT role FROM container_member WHERE container_id = $1 AND user_id = $2",
        container_id,
        new_owner_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(new_owner_role) = new_owner_role else {
        return Err(DomainError::NotFound.into());
    };

    sqlx::query!(
        "UPDATE container SET owner_id = $2 WHERE id = $1",
        container_id,
        new_owner_id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE container_member SET role = 'owner' WHERE container_id = $1 AND user_id = $2",
        container_id,
        new_owner_id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE container_member SET role = $3 WHERE container_id = $1 AND user_id = $2",
        container_id,
        current_owner_id,
        new_owner_role
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(())
}
