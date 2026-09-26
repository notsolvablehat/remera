use sqlx::PgPool;
use uuid::Uuid;

pub struct AllowlistEntry {
    pub email: String,
    pub claimed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Re-adding an email that was already claimed and then left/was removed
/// must actually re-arm the invite — `DO NOTHING` would silently leave
/// `claimed_at` set from the previous claim, permanently blocking
/// re-resolution for that email (see backend/AGENTS.md's "Discovered
/// gap" note this fixes).
pub async fn insert(pool: &PgPool, container_id: Uuid, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO container_edit_allowlist (container_id, email) VALUES ($1, $2)
         ON CONFLICT (container_id, email) DO UPDATE SET claimed_at = NULL",
        container_id,
        email
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_for_container(
    pool: &PgPool,
    container_id: Uuid,
) -> Result<Vec<AllowlistEntry>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT email, claimed_at FROM container_edit_allowlist
         WHERE container_id = $1
         ORDER BY created_at DESC",
        container_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| AllowlistEntry {
            email: r.email,
            claimed_at: r.claimed_at,
        })
        .collect())
}

pub async fn delete(pool: &PgPool, container_id: Uuid, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM container_edit_allowlist WHERE container_id = $1 AND email = $2",
        container_id,
        email
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Grants Editor access for every unclaimed allow-list entry matching
/// `email`, and marks each entry claimed. Idempotent: `ON CONFLICT DO
/// NOTHING` on the membership insert means re-running this for an
/// already-resolved user is a cheap no-op.
pub async fn resolve_pending_edit_invites(
    pool: &PgPool,
    user_id: &str,
    email: &str,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, container_id FROM container_edit_allowlist
         WHERE email = $1 AND claimed_at IS NULL",
        email
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    for row in rows {
        sqlx::query!(
            "INSERT INTO container_member (container_id, user_id, role) VALUES ($1, $2, 'editor')
             ON CONFLICT (container_id, user_id) DO NOTHING",
            row.container_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "UPDATE container_edit_allowlist SET claimed_at = NOW() WHERE id = $1",
            row.id
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
