# Lessons

General rules learned the hard way. Each is short, factual, and names its evidence in this
repository. Add one only when the same class of mistake has happened more than once.

## A proved component is not a usable product path

High-quality protocol, browser and record proofs can accumulate while the product's own first run
lands in an under-composed shell. Every product milestone needs a front-door acceptance walk.

## Independence is a capability, not an onboarding requirement

A headless server and stock MCP access are useful. Requiring a person to compose those pieces by
hand is not a virtue. The product assembly may be opinionated while components stay independently
usable.

## Preserve old behaviour explicitly during rewrites

If an earlier implementation could perform a useful action, a rewrite preserves, replaces or
retires it deliberately, or architectural purity silently deletes product capability.

## A product gate is phrased as a person's action, or it is not a product gate

A gate that reads "the surface mounted" or "the digests match" is a component test. Every product
gate is written as something a person did — installed, configured, said, heard — and a gate that
cannot be read that way does not count toward a product milestone.

## A capability is proven by its journey, never by its slice

A capability moves to `proven` only when the journey it belongs to passes end to end through the
product's entry point. Until then it is `partial`, and the row names the missing halves.

## A driver waits for a signal the page's script sets, never for markup

A readiness attribute present in static markup is read before the script has loaded. Set it from
the script; and after placing files in an input through the browser protocol, dispatch `change`
from the page's own world when the page has not consumed them.

## Wait for what a person is left with, not for the report of the thing happening

A progress line, a toast and a spinner belong to the frame they were drawn in and are swept by
the next render. A walk waits for the state the gesture leaves behind: the row on the page, the
file on disk.

## A React select reads back its old value the moment you set it

A controlled field renders the state, and the state moves only when the change lands. In a
driver, assert the value you intended was accepted, then wait for the page to show it.

## A gate that picks one of several answers reports luck

Where a helper must choose between answers that cannot both be right, it fails and names them.
A gate may be red or green; it may not be lucky.

## A flaky gate diagnosed from one run is a guess wearing evidence's clothes

Before writing a cause down, change one variable and rerun, and run the suspect walk twice on the
unchanged tree. If it does not fail the same way both times, write "not established" and what
would establish it.

## A red nobody questions becomes a fact

An accepted red is a claim like any other and it decays. When one has a name rather than a cause
("no device here", "flaky", "environment"), measure it again from the page before repeating it.

## A walk can prove the capability and never touch the way in

When a walk reaches past a control to the thing behind it — an input behind a button, a handler
behind a form — that reach is the gap. Press the control; keep the reach for what the control
cannot do.

## A read that only labels a list must not dial the thing it labels

A list answers from what listing already knows. A field only the thing itself can tell you is
filled when that thing is opened and absent until then. A page load that starts a child process
per row is the symptom.

## A fence nobody runs is not a fence

Suites that are "run them yourself" are run by whoever remembers. `scripts/suites.sh` and
`scripts/gate.sh` name what they run and skip nothing by name; a `--skip` word is a substring and
one word can hide several tests, so anchor it and check the count.

## A fixture binary another crate's tests run is an example, found beside the product binary

A `[[bin]]` may only use its crate's normal dependencies; an `example` may use dev-dependencies.
`CARGO_BIN_EXE_<name>` is set only for a crate's own binaries, so a shared fixture module locates
other binaries beside its own anchor.

## The node suites import `.ts` and need type stripping on Node older than 22.18

`ERR_UNKNOWN_FILE_EXTENSION ".ts"` on a tree where nothing under the page changed is a missing
`--experimental-strip-types`, which `scripts/suites.sh` sets for the Node that needs it.

## A walk waits for what a person sees, not for what the record says

The page learns that a turn ended a moment after the ledger does. A driver that read the ledger
and pressed at once pressed a button still disabled, and nothing happened. Wait for the control
to be offered (`putToSleep` in `cdp_browser.mjs`), then press.

## Two counters that both start small are one bug waiting

The panel of Apps compared the place of an event in the ledger with the number of a call in a
session. Both are integers and the first App worked, so nothing failed until a second one was
brought. A number that crosses a boundary carries its name with it (`place`, `call`).

## A suite that was not run is not green

`genericity` anchored its scan on a file of the old page and was red for four commits of this
slice because only the suites a change "touches" were run. Before a slice ends, run
`scripts/suites.sh` whole, once.
