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

A person chooses in an agent's Settings the provider and the model it answers from, writes what
it is for, gives it skills and servers by name; what opens a provider is given in Providers
(GJ-10); the agent gets them in its own layout before launch, and the session's option
shows the model. Walk: `product_front_door::a_person_gives_an_agent_a_role_and_a_model`.

## GJ-10 — Providers: a key given once, and what this machine offers

A person opens Providers from the rail. Under Models they add a place a model is served from
and give it what opens it, once; an agent that answers from it is handed it when it starts and
nobody types it into the agent. Under Hosts they read what the machine SWEM runs on has, look
again when they like, and see where on it an agent may live; a place that cannot take an agent
today says why and is not offered in an agent's settings. Walk:
`product_front_door::a_person_gives_an_agent_a_role_and_a_model` (the key); Hosts by hand and by
`hosts::where_an_agent_may_live_is_offered_from_what_was_found`.

## GJ-11 — A message that arrives on time

A person opens an agent's Schedules and makes a schedule: what the agent is told, when, and the
chat it is said in. It is said on time by the schedule and answered like anything else said
there, with nobody watching. They pause it with its switch and forget it; what it did stays.
They read the last runs. Asked to check back, the agent makes a schedule itself, and the
person switches it off. Providers says who keeps time. Walk:
`product_front_door::a_person_leaves_a_standing_instruction_and_the_product_carries_it_out`.

## GJ-03 — Install from the Store

A person opens the Store, adds a catalog by address, installs an agent, an MCP server and a skill,
each behind its own consent dialog, gives the agent both, and the agent calls the server's tool
and reads the skill. Walk: `product_front_door::a_person_installs_from_the_store_and_the_agent_uses_it`.

## GJ-04 — Chats kept across restarts

An agent's chats are listed, the one that moved last first; picking one shows everything that
was said with each message under its sender, and the next message goes on in it; after the
product restarts they are still there, and so is a question nobody answered yet. A chat from
before chats existed is there too. Evidence: by hand on a copy of a real data root with a real
engine, the product restarted in between (ExecPlan 005); `chat_ledger`, `chat_runtime`;
`product_front_door::a_person_closes_their_editor_and_comes_back_to_the_same_conversation`.

## GJ-09 — Several agents at once, and who said what

A person talks to one agent while another works or waits for an answer; stops a turn from the
composer; closes the page while an agent asks before it does something, opens it again and
answers. Every message has its sender. Evidence: by hand, as above;
`chat_runtime::two_agents_work_at_once_and_a_question_waits_for_whoever_comes_back` and its
siblings. No browser walk repeats it by itself yet.

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
Status: done (roadmap C4;
`product_front_door::a_person_installs_a_server_with_a_home_app_and_opens_its_space`,
`chat_runtime::what_an_app_said_the_person_is_looking_at_goes_with_their_words`). The host
draws no Project space of its own.
