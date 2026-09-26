use domain::Role;
use sqlx::PgExecutor;
use uuid::Uuid;

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
