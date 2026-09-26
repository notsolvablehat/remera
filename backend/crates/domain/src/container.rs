use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Viewer = 0,
    Editor = 1,
    Owner = 2,
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
