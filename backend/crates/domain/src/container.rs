use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A container owner may own at most this many containers at once
/// (free-tier quota — see backend/AGENTS.md "Design decisions already made").
pub const MAX_OWNED_CONTAINERS: i64 = 5;

#[derive(Debug, Clone)]
pub struct Container {
    pub id: Uuid,
    pub owner_id: String,
    pub name: String,
    pub is_public: bool,
    pub is_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Viewer = 0,
    Editor = 1,
    Owner = 2,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Owner => "owner",
        }
    }
}

impl FromStr for Role {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "editor" => Ok(Role::Editor),
            "owner" => Ok(Role::Owner),
            _ => Err(()),
        }
    }
}

pub fn role_meets_minimum(actual: Role, required: Role) -> bool {
    actual >= required
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_can_do_everything() {
        assert!(role_meets_minimum(Role::Owner, Role::Viewer));
        assert!(role_meets_minimum(Role::Owner, Role::Editor));
        assert!(role_meets_minimum(Role::Owner, Role::Owner));
    }
    #[test]
    fn viewer_cannot_edit() {
        assert!(!role_meets_minimum(Role::Viewer, Role::Editor));
    }
    #[test]
    fn editor_cannot_own() {
        assert!(!role_meets_minimum(Role::Editor, Role::Owner));
    }
}
