//! Hosts: the machines agents live on, and what was found when each was
//! looked at.
//!
//! Nothing is offered that was not looked at. This module holds the first
//! look, at the machine the Workbench itself runs on.

pub mod system_scheduler;
mod this_machine;

pub use this_machine::{ContainerEngineLook, EngineStanding, MachineLook};
pub(crate) use this_machine::{look_at_this_machine, read_look, write_look};
