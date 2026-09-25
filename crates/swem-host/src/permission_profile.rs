//! How an agent's permission requests are answered, as a thing a person picks.
//!
//! ACP agents ask before they act. Until now the Workbench answered every
//! request the same way - by showing it to the person - which is right when
//! somebody is watching and useless when nobody is: an agent left to work on
//! its own stops at the first question and waits forever.
//!
//! So a profile says which of these it uses, and the person picks it from a
//! list rather than by naming an internal policy. Nothing here widens what an
//! agent may do by default: a profile that says nothing keeps [`ASK_EVERY_TIME`],
//! which is what every profile did before this existed.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{SeamPermissionPolicy, SessionPermissionPolicy};

/// Every request goes to the person, and the agent waits for an answer.
pub const ASK_EVERY_TIME: &str = "surface-permissions";

/// The agent decides for itself, inside a stated boundary: it reads and
/// searches freely, it changes files inside its own working directory, and it
/// calls the tools of the MCP servers the profile attached. Anything else -
/// a shell command, a file outside the working directory, an MCP server this
/// profile did not attach - is refused rather than asked.
pub const INSIDE_ITS_WORKSPACE: &str = "workspace-permissions";

/// One choice, as the surface shows it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PermissionProfileOption {
    pub permission_profile_id: String,
    /// What the person reads in the list.
    pub name: String,
    /// What picking it means, in the same words.
    pub summary: String,
}

/// The choices this host offers, in the order a person should meet them.
#[must_use]
pub fn permission_profiles() -> Vec<PermissionProfileOption> {
    vec![
        PermissionProfileOption {
            permission_profile_id: ASK_EVERY_TIME.into(),
            name: "Ask me every time".into(),
            summary: "The agent stops and asks before anything that changes something, and \
                      before every command it wants to run. Right when you are watching it \
                      work."
                .into(),
        },
        PermissionProfileOption {
            permission_profile_id: INSIDE_ITS_WORKSPACE.into(),
            name: "Work on its own, inside its working directory".into(),
            summary: "The agent reads and searches freely, changes files inside its working \
                      directory, runs commands there, and uses the servers you attached. A file \
                      outside that directory, or a server you did not attach, is refused, not \
                      asked. A command is not sandboxed: it runs on this machine as you do. \
                      Right when you are not there."
                .into(),
        },
    ]
}

/// The policy a session runs under, from what the profile says.
///
/// `workspace` is the directory the profile works in and `attached` the MCP
/// servers it attached; both come from the profile rather than from a caller,
/// so picking a choice cannot widen the boundary it names.
///
/// # Errors
///
/// Returns the id back when this host offers no such choice, rather than
/// falling back to something the person did not pick.
pub fn permission_policy(
    permission_profile_id: &str,
    workspace: &Path,
    attached: Vec<String>,
) -> Result<SessionPermissionPolicy, String> {
    match permission_profile_id {
        ASK_EVERY_TIME => Ok(SessionPermissionPolicy::Surface),
        INSIDE_ITS_WORKSPACE => Ok(SessionPermissionPolicy::SwemSeam {
            policy: SeamPermissionPolicy {
                mcp_server_names: attached,
                allow_read_only_tools: true,
                authoring_workspace: Some(workspace.to_path_buf()),
            },
        }),
        other => Err(format!("this host offers no permission profile {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{ASK_EVERY_TIME, INSIDE_ITS_WORKSPACE, permission_policy, permission_profiles};
    use crate::SessionPermissionPolicy;

    #[test]
    fn every_choice_offered_is_a_choice_that_resolves() {
        for option in permission_profiles() {
            permission_policy(
                &option.permission_profile_id,
                std::path::Path::new("/tmp"),
                Vec::new(),
            )
            .unwrap_or_else(|error| {
                panic!("{} is offered but does not resolve: {error}", option.name)
            });
        }
    }

    #[test]
    fn working_alone_is_bounded_by_the_profiles_own_workspace_and_attachments() {
        let policy = permission_policy(
            INSIDE_ITS_WORKSPACE,
            std::path::Path::new("/tmp/somewhere"),
            vec!["a-project".to_owned()],
        )
        .expect("the choice resolves");
        let SessionPermissionPolicy::SwemSeam { policy } = policy else {
            panic!("working alone must not be the surface policy");
        };
        assert_eq!(
            policy.authoring_workspace.as_deref(),
            Some(std::path::Path::new("/tmp/somewhere"))
        );
        assert_eq!(policy.mcp_server_names, ["a-project"]);
    }

    #[test]
    fn asking_every_time_stays_the_surface_policy() {
        assert_eq!(
            permission_policy(ASK_EVERY_TIME, std::path::Path::new("/tmp"), Vec::new()),
            Ok(SessionPermissionPolicy::Surface)
        );
    }

    #[test]
    fn a_choice_this_host_does_not_offer_is_refused_rather_than_guessed() {
        assert!(
            permission_policy("allow-everything", std::path::Path::new("/tmp"), Vec::new())
                .is_err()
        );
    }
}
