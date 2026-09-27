# Golden journeys

What a person must be able to do with SWEM's Workbench, each through a real entry point. A
capability is `proven` only when its journey passes end to end through that entry point; the walk
that proves it is named.

## GJ-01 — First run: install an agent and talk to it

A person with nothing starts `swem`, sees the agents this computer has or can install, installs
one against the exact plan the page shows, makes it theirs, chooses how it asks for permission,
starts a session, says something and gets the agent's own answer; opens a terminal in the agent's
environment; declares a server of their own and attaches it; adds a second profile of the same
agent. Walk: `product_front_door::a_person_with_nothing_installs_an_agent_and_can_start_it`.

## GJ-02 — Set up an agent: model, role, skills, keys, servers

A person gives a profile a model provider and a model, a role and skills in Setup, keys under Keys,
MCP servers by name; the agent gets them in its own layout before launch, and the session's option
shows the model. Walk: `product_front_door::a_person_gives_an_agent_a_role_and_a_model`;
`workbench_agent_setup_browser`.

## GJ-03 — Install from the Store

A person opens the Store, adds a catalog by address, installs an agent, an MCP server and a skill,
each behind its own consent dialog, gives the agent both, and the agent calls the server's tool
and reads the skill. Walk: `product_front_door::a_person_installs_from_the_store_and_the_agent_uses_it`.

## GJ-04 — Sessions kept across restarts

A profile's sessions are listed newest first; picking one resumes its conversation; after the
product restarts they are still there. Walk: `workbench_shell_sessions_browser`.

## GJ-05 — An agent in a container

A person puts their agent in a container on this machine and everything still works: the session,
the files, the terminal, the model. Walk: `product_front_door::a_person_puts_their_agent_in_a_container_and_it_still_works`.

## GJ-06 — From an editor over ACP

A person points an editor that speaks ACP at `swem acp --profile <id>` and works with the same
agent, setup and sessions as the page; the agent reads and writes the file the person has open;
closing and reopening the editor returns to the same conversation. Walks:
`product_front_door::a_person_works_with_their_agent_from_their_editor` and its siblings.

## GJ-07 — Embed the harness in another application

An application starts a Workbench through the crate's product builder in a page of code and, with
a client credential, drives a session from a page of its own on another origin. Status: the
builder and the example are done (roadmap C1; `tests/product::an_embedder_serves_a_workbench_
from_a_page_of_code`); the client credential is planned (roadmap C2).

## GJ-08 — A project server's own space

A server installed from the Store that declares a home App appears as a space; opening it shows
the App; what the App says a person is looking at is given to the agent with the next turn.
Status: done (roadmap C4; `workbench_shell_spaces_browser`, `workbench_shell_context_browser`,
`product_front_door::a_person_installs_a_server_with_a_home_app_and_opens_its_space`). The host
draws no Project space of its own.
