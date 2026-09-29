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

An agent is made in four steps: what it is built on and who it is - a name, a handle, a colour,
what it is for; where it works, its model and who keeps its time, each chosen from what was set
up and looked at, a host that cannot take an agent saying why; how it signs in and what it will
have where it lives, looked at from inside; what it may do. It can be made after the second.
An engine that is not there is installed from the first step, by the plan the page shows in a
dialog of its own. Every walk of the gate that makes an agent takes the first two steps; the
four were done by hand with a real engine.

## GJ-02 — Set up an agent: model, role, skills, keys, servers

A person chooses in an agent's Settings the provider and the model it answers from, writes what
it is for, gives it skills and servers by name; what opens a provider is given in Providers
(GJ-10); the agent gets them in its own layout before launch, and the session's option
shows the model. Walk: `product_front_door::a_person_gives_an_agent_a_role_and_a_model`.

An agent a person no longer wants is removed in its Settings, under Remove, after what goes and
what stays is read: it is stopped and is nobody to write to; its chats stay with what it said,
under its name; its schedules are forgotten and what it held of its own with them; the folder
it worked in stays on the disk and the page says where; its name and handle can be given to a
new agent, which starts with no chats. Under the agent's Channels a person reads the doors into
its chats - the Workbench, an editor, by the command to copy - and who may write to it. Proven
by `removal::an_agent_is_removed_and_what_it_said_stays` and by hand.

## GJ-10 — Providers: a key given once, and what this machine offers

A person opens Providers from the rail. Under Models they add a place a model is served from
and give it what opens it, once; an agent that answers from it is handed it when it starts and
nobody types it into the agent. Under Hosts they read what the machine SWEM runs on has, look
again when they like, and see where on it an agent may live; a place that cannot take an agent
today says why and is not offered in an agent's settings. Walk:
`product_front_door::a_person_gives_an_agent_a_role_and_a_model` (the key); Hosts by hand and by
`hosts::where_an_agent_may_live_is_offered_from_what_was_found`.

Providers has Engines as well - the coding agents this computer has, installed from there, with
the agents that stand on each - and Channels, the doors into chats. "Add a host" says which
kinds there are and that none other than this machine is reached yet.

Where Podman is there and has no machine, Set up beside it says what would be done - the machine
and its size, what is downloaded, the two commands - and what this computer cannot give; agreed
to as offered, it is done step by step where the person sees it, the machine is looked at again,
and a failure is said in Podman's own words with what to do. Proven by
`hosts::containers_are_set_up_by_the_plan_a_person_agreed_to`; by hand up to the refusal on a
computer where QEMU does not start, and with Podman's machine commands stood in for. Not yet
done on a computer where the machine really starts.

## GJ-11 — A message that arrives on time

A person opens an agent's Schedules and makes a schedule: what the agent is told, when, and the
chat it is said in. It is said on time by the schedule and answered like anything else said
there, with nobody watching. They pause it with its switch and forget it; what it did stays.
They read the last runs. Asked to check back, the agent makes a schedule itself, and the
person switches it off. Providers says who keeps time. Walk:
`product_front_door::a_person_leaves_a_standing_instruction_and_the_product_carries_it_out`.

Under Providers, Time a person reads who can keep time and what each cannot do. They turn the
system's own scheduler on, after reading what is put on the computer, and choose in an agent's
Schedules who keeps its time. With the Workbench closed the computer starts SWEM when
something of that agent's is due; it is said and answered, and SWEM leaves. Opened again, the
Workbench keeps time itself and Time says when the scheduler looked last. Turned off, nothing
of it is left on the computer. Proven by
`chat_runtime::time_is_kept_once_for_the_agents_of_a_keeper`, `keepers`, and by hand with
launchd and a real engine; a systemd timer is written and not yet done on a machine.

## GJ-12 — A chat of several

A person starts a chat with two agents from the rail. What they write is for who it names: they
pick a name after `@`, the one named answers and the other does not, and what named nobody
says so. An agent that asks another by name is told its answer. Beside the chat they see who
is in it and what each is doing, bring an agent in and take one out, and set whether agents
answer only when named and after how many replies to each other the chain waits for a person.
Walk: `product_front_door::a_person_puts_two_agents_in_one_chat`.

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

## GJ-13 — An agent's files

A person opens an agent's Files and sees the folder it works in as a tree. They open a file and
read it coloured, change it and save it, and the agent reads what they wrote. When the agent
wrote the file meanwhile they are told, nothing is saved, and they choose: read it again or
save theirs over it. Leaving a file that was changed and not saved asks first. They make a file
and a folder, rename and remove, and bring files from their own machine into a folder; one whose
name is taken is replaced only when they say so. A picture is shown; what else is not text is
opened as it is. What they handed over and
what the agent handed back are listed beside the tree. Proven by `swem-runner`'s `fs`,
`agent_files`, and by hand with a real engine; the lists of what was handed over and back by
`product_front_door::a_person_hands_their_agent_a_file_and_gets_one_back`.

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
