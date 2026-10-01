# SWEM Extension Exception

Additional permission under section 7 of the GNU Affero General Public License, version 3.

As a special exception to the AGPL, the author of SWEM grants you the following additional
permission.

A work that interacts with SWEM only through its extension interfaces is not a covered work by
reason of that interaction alone, and you may convey it under terms of your choice - proprietary
terms included - provided those terms do not restrict the Program itself. The extension interfaces
are:

- the Agent Client Protocol, as spoken by an agent SWEM runs;
- the Model Context Protocol and MCP Apps, as spoken by a server SWEM hosts, including a server
  that takes packages for itself;
- a catalog, a skill or a package that the Store installs, in the formats SWEM documents;
- the package interface of the Cycle: its manifest, its component interface and its adapters;
- the public interfaces of `swem-sdk`, and of any other crate or package this repository marks
  as an SDK.

This permission covers a work that is conveyed as a separate artifact and reaches SWEM through
those interfaces. It does not cover a work that includes SWEM's own code, or that links SWEM's
crates into itself other than an SDK (`swem-sdk` is Apache-2.0 in its own right): such a work is a covered work under the AGPL, or needs a
licence from the author.

If you modify SWEM, you may keep this exception on your modified version or remove it; you may
not widen it.
