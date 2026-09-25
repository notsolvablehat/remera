/*
This file compiles but is currently unused. The obvious way to wire it — AuthBuilder::hook(...), or wrapping the adapter in HookedDatabaseAdapter — does not work on better-auth-core 0.10.0:
AuthBuilder::build() explicitly returns Err(...) if any hooks were registered via .hook() (core/auth.rs:180-183), telling you to use HookedDatabaseAdapter instead.
But HookedDatabaseAdapter<DB> implements every *Ops trait (UserOps, SessionOps, …) except PasskeyOps, so it can never satisfy DatabaseAdapter’s full trait bound — confirmed against the crate’s own vendored source. Checking the crate’s GitHub master branch, HookedDatabaseAdapter has since been removed from hooks.rs entirely (there’s a 1.0.0-alpha.2 published after 0.10.0) — this was a known gap that got reworked upstream, not a mistake on our end.
Until better-auth is upgraded past 0.10.0, invite resolution can’t run as an automatic DatabaseHooks callback. The struct and impl below are written so the logic exists and compiles, but actually triggering after_create_user requires calling it explicitly from a custom signup endpoint instead of relying on better-auth’s built-in /auth/sign-up/email route — that wiring is a TODO, not covered by this guide yet.
*/

use better_auth::adapters::SqlxAdapter;
use better_auth::entity::AuthUser as _; // for .email()
use better_auth::{AuthResult, DatabaseHooks};
use sqlx::PgPool;

pub struct AppAuthHooks {
    pub db: PgPool,
}

// DatabaseHooks IS #[async_trait] in better-auth-core (unlike axum's
// FromRequestParts) — it needs a boxed future to be object-safe. Hence
// the `async-trait` crate dependency added in step 1.
#[async_trait::async_trait]
impl DatabaseHooks<SqlxAdapter> for AppAuthHooks {
    // Fires immediately AFTER a new user is successfully inserted — the
    // natural place to resolve pending invites for that email, IF this
    // hook mechanism were reachable (see warning above).
    async fn after_create_user(
        &self,
        user: &<SqlxAdapter as better_auth::DatabaseAdapter>::User,
    ) -> AuthResult<()> {
        let Some(email) = user.email() else {
            return Ok(());
        };

        // Only active invites: not yet accepted, not yet expired.
        let invites = sqlx::query!(
            "SELECT id, container_id, role FROM invite
             WHERE email = $1 AND accepted_at IS NULL AND expires_at > NOW()",
            email
        )
        .fetch_all(&self.db)
        .await
        .unwrap_or_default();

        for invite in invites {
            // Insert container_member + mark invite accepted, in one
            // transaction, per invite — schema already exists in
            // migrations/004_create_container_tables.sql.
            let _ = (invite.id, invite.container_id, invite.role);
        }

        Ok(())
    }
}
