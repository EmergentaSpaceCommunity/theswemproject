//! Where a profile's agent runs, as a thing a person picks.
//!
//! A profile has always carried an environment by name, and nothing ever read
//! it: whatever the name said, the product started the agent as a process on
//! this machine. The name was a string with nothing behind it, so a profile
//! could state one thing while the product did another, and the person had no
//! way to see the difference.
//!
//! So the environments this host offers become a list, the same shape as the
//! permission choices next to it: a person picks one, the page shows what it
//! means, and a profile naming an environment this host does not have is
//! refused rather than quietly started here anyway.
//!
//! The list holds two entries: the machine itself, and a container on it. The
//! second is where a person puts an agent they would rather not have reading
//! everything they own - it gets the profile's directory and nothing else of
//! the machine, and no network. Whether this host can actually run one is a
//! separate question from whether it is offered: the product asks its own
//! probe when a profile names it, and says what is missing rather than
//! starting the agent on the machine instead.

use serde::{Deserialize, Serialize};

/// The agent is a process on the person's own machine, in the profile's
/// working directory. This is the id every profile written before this
/// existed already carries, so nothing has to be migrated.
pub const THIS_MACHINE: &str = "direct-host-environment";

/// The agent runs in a container on the person's own machine, with the
/// profile's working directory bound into it and nothing else of the machine
/// visible.
pub const IN_A_CONTAINER: &str = "podman-container-environment";

/// How an environment actually runs an agent. What the resolver switches on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentBackend {
    /// A child process of the product, on this machine.
    ThisMachine,
    /// A container on this machine, prepared by the product before the agent
    /// starts and torn down when it stops.
    InAContainer,
}

/// One environment, as the surface shows it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentProfileOption {
    pub environment_profile_id: String,
    /// What the person reads in the list.
    pub name: String,
    /// What picking it means, in the same words.
    pub summary: String,
}

/// The environments this host offers, in the order a person should meet them.
#[must_use]
pub fn environment_profiles() -> Vec<EnvironmentProfileOption> {
    vec![
        EnvironmentProfileOption {
            environment_profile_id: THIS_MACHINE.into(),
            name: "On this machine".into(),
            summary: "The agent runs as a program on your computer, in its working directory, \
                      with whatever your machine has installed."
                .into(),
        },
        EnvironmentProfileOption {
            environment_profile_id: IN_A_CONTAINER.into(),
            name: "In a container on this machine".into(),
            summary: "The agent runs inside a container that holds its working directory and \
                      nothing else of your computer, with no network. Your machine needs Podman \
                      and the image this product runs agents in."
                .into(),
        },
    ]
}

/// How an environment runs an agent, from what the profile says.
///
/// # Errors
///
/// Returns the id back when this host offers no such environment, rather than
/// falling back to running the agent somewhere the profile did not name.
pub fn environment_backend(environment_profile_id: &str) -> Result<EnvironmentBackend, String> {
    match environment_profile_id {
        THIS_MACHINE => Ok(EnvironmentBackend::ThisMachine),
        IN_A_CONTAINER => Ok(EnvironmentBackend::InAContainer),
        other => Err(format!("this host offers no environment {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EnvironmentBackend, IN_A_CONTAINER, THIS_MACHINE, environment_backend, environment_profiles,
    };

    #[test]
    fn every_environment_offered_is_one_that_resolves() {
        for option in environment_profiles() {
            environment_backend(&option.environment_profile_id).unwrap_or_else(|error| {
                panic!("{} is offered but does not resolve: {error}", option.name)
            });
        }
    }

    #[test]
    fn the_environment_every_existing_profile_names_is_this_machine() {
        assert_eq!(
            environment_backend(THIS_MACHINE),
            Ok(EnvironmentBackend::ThisMachine)
        );
    }

    #[test]
    fn the_two_environments_are_different_ways_of_running_an_agent() {
        assert_eq!(
            environment_backend(IN_A_CONTAINER),
            Ok(EnvironmentBackend::InAContainer),
            "the container row must not resolve to running on the machine"
        );
    }

    #[test]
    fn an_environment_this_host_does_not_offer_is_refused_rather_than_guessed() {
        let refusal = environment_backend("a-container-somewhere")
            .expect_err("an unknown environment must not resolve");
        assert!(
            refusal.contains("a-container-somewhere"),
            "the refusal must name what was asked for: {refusal}"
        );
    }
}
