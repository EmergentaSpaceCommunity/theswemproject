//! What SWEM puts on a host to reach an agent's machine.
//!
//! The product asks and the runner answers, the same way wherever the
//! machine is: on the computer the product runs on the library answers in
//! the product's own process; in a machine of the agent's own the binary
//! answers on its standard streams. There is one way to reach an agent's
//! files because of that, and no second one for a container.

pub mod exec;
pub mod fs;
pub mod probe;

pub use exec::{ExecHeader, RUNNER_EXEC_SCHEMA, become_it, header_at, header_from};
pub use fs::{Entry, EntryKind, FsAnswer, FsRequest, RUNNER_FS_SCHEMA, Why, answer, digest_of};
pub use probe::{ProbeAnswer, ProbeRequest, ProgramFound, RUNNER_PROBE_SCHEMA, look};
