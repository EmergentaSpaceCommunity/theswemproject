#!/usr/bin/env python3
"""Which named controls of a workbench's pages no walk has ever touched.

    unwalked_controls.py [pages-directory [walk-directory ...]]

With no arguments it counts the Workbench's own pages against every walk. A
domain workbench's pages are given as arguments; the checks still run against
the Workbench's, so a broken search is caught either way.

The number this prints gets quoted, and the hand-rolled greps that preceded
it were wrong three times over. So it lives here, with each mistake encoded as
a check it makes before it counts:

* a driver reaches a control by `#id`, by `getElementById("id")` or by a data
  attribute, so the search is for the bare name and not for a selector;
* a JSX opening tag is routinely several lines, so the markup is parsed
  across newlines rather than a line at a time;
* a control need not carry an `id` at all - a literal `className` or a
  `data-` attribute is a name a driver takes it by, and counting ids alone
  hid a fifth of them;
* a name counts only where it could be a name - after a selector's `#` or
  `.`, or inside a string. A driver explains itself in English, and a
  control named for an ordinary word (`edge`, `lane`, `section`, `in`) is
  otherwise pressed by every comment that happens to use it;
* only a control's own name vouches for it. A control carries names it
  shares with others, and letting any of them answer marks a control nobody
  presses as pressed. The music thread has a live one: a fade handle nobody
  drags, carried by the crossfade walk's `.fade.in`.

The last two came from the music thread, which measures the same thing on
the other half of the pages; both were watched failing before they were
kept, and neither moved this number.

A control need not be a control tag either: a span a person drags by is one.
An element carrying a handler a person triggers is counted as a control and
printed with a `*` on its tag.

A control whose every name is computed (`className={`row ${kind}`}`) carries
no name a walk could use. Those are counted apart: not untouched so much as
untouchable without editing the page first.

What it prints is a list of candidates, not a verdict: a driver can reach a
control through its text, its position or a computed attribute and never name
it, so each name here is settled by reading the walk that might press it. Nothing below that can be measured except
by walking each one.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# The Workbench's own pages, and everywhere a walk lives.
PAGES = ROOT / "crates/swem-host/web/apps-host/src"
WALKS = [ROOT / "crates" / crate / "tests" for crate in ("swem-host", "swem-cli", "swem-cycle")]
CONTROLS = {"input", "button", "select", "textarea"}
# A control is not always a control tag. A span a person drags by, a div that
# takes a click - the music workbench's fade handles are spans - are controls
# to whoever uses the page, so an element that carries a handler a person
# triggers counts as one whatever its tag. `onSubmit` is left out on purpose:
# a form is the container, and the button inside it is the control.
HANDLER = re.compile(r"\bon(Click|Change|Input|PointerDown|MouseDown|KeyDown|Drop)\s*=")
ELEMENT = re.compile(r"<([a-zA-Z]+)((?:[^<>]|\{[^{}]*\})*?)>", re.S)
IDENTIFIER = re.compile(r'\bid="([a-zA-Z0-9_-]+)"')
LITERAL_CLASS = re.compile(r'\bclassName="([^"{}]+)"')
# A template class carries literal words too: `className={`row ${kind}`}` is
# reachable as `row`. The holes are dropped - on another render they are
# another class - and what is left is a name a walk can ask for. The music
# thread's script found controls this way that counting the quoted form
# alone had missed.
TEMPLATE_CLASS = re.compile(r"\bclassName=\{`([^`]*)`\}")
HOLE = re.compile(r"\$\{[^{}]*\}")
# Data attributes are deliberately NOT names here. The ones a walk targets
# carry computed values (`data-server={server.name}`), so only the bare
# attribute could be matched - and it sits on the row as well as on the
# button inside it, which would mark a button nobody presses as pressed.
# Counting them made the list quieter and less true, so what is printed is a
# list of candidates: a control reachable only through a data attribute shows
# up here and is settled by reading the walk, not by the count.


def named_elements(pages: Path) -> dict[str, tuple[str, tuple[str, ...]]]:
    """Every element a walk could name, by its first name, with its tag and
    every name it carries. An element with no literal name of its own is left
    out; `unnameable` counts those."""
    found: dict[str, tuple[str, tuple[str, ...]]] = {}
    for page in sorted(pages.rglob("*.tsx")):
        for tag, names in _elements_of(page.read_text()):
            found.setdefault(names[0], (tag, names))
    return found


def _elements_of(source: str) -> list[tuple[str, tuple[str, ...]]]:
    """Every named element of one source, as (tag, names). A tag ending in
    `*` is not a control tag but carries a handler a person triggers."""
    elements = []
    for tag, attributes in ELEMENT.findall(source):
        if tag not in CONTROLS and HANDLER.search(attributes):
            tag = f"{tag}*"
        names = []
        identifier = IDENTIFIER.search(attributes)
        if identifier:
            names.append(identifier.group(1))
        literal = LITERAL_CLASS.search(attributes)
        if literal:
            names.extend(literal.group(1).split())
        template = TEMPLATE_CLASS.search(attributes)
        if template:
            names.extend(HOLE.sub(" ", template.group(1)).split())
        if names:
            elements.append((tag, tuple(names)))
    return elements


def unnameable(pages: Path) -> int:
    """Controls with no literal name at all: a walk cannot ask for them."""
    return sum(_nameless_of(page.read_text()) for page in sorted(pages.rglob("*.tsx")))


def _nameless_of(source: str) -> int:
    count = 0
    for tag, attributes in ELEMENT.findall(source):
        acts = tag in CONTROLS or bool(HANDLER.search(attributes))
        if not acts or IDENTIFIER.search(attributes) or LITERAL_CLASS.search(attributes):
            continue
        template = TEMPLATE_CLASS.search(attributes)
        if template and HOLE.sub(" ", template.group(1)).split():
            continue
        count += 1
    return count


SAMPLE = """
      <input
        id="sample-field"
        placeholder="named across two lines"
      />
      <button className="sample-plain">by a quoted class</button>
      <button className={`sample-template ${chosen} sample-tail`}>by a template class</button>
      <span className="sample-handle" onPointerDown={drag}>a control that is not a control tag</span>
      <div className="sample-box">not a control at all</div>
      <button onClick={go}>no name at all</button>
"""


# Where a name can be a name: right after a selector's `#` or `.`, or inside
# a string - `"#packages > summary"`, `getElementById("prompt-text")`,
# `[data-tool="swem-stub-tool"]`. An English sentence in a driver's comment
# that happens to use the word (`the far edge of the rail`, `one more turn in
# the lane`) is prose about a walk, not the walk. Whole names only after
# that: `rec` must not be found inside `record`.
NAME_HERE = re.compile(r"""[#.="'`\[]""")


def touched_in(haystack: str, names: tuple[str, ...]) -> bool:
    """The one place a control is looked for, so the checks are checks of it.

    Only a control's OWN name vouches for it - `names[0]`, its id or the
    first word of its class. A control carries names it shares with others
    (`<span className="edge in">` carries `in`), and letting any of them
    answer marks a control nobody presses as pressed: the music thread has a
    live one, a fade handle nobody drags that the crossfade walk's `.fade.in`
    would have vouched for.
    """
    name = names[0]
    for match in re.finditer(
        rf"(?<![A-Za-z0-9_-]){re.escape(name)}(?![A-Za-z0-9_-])", haystack
    ):
        before = haystack[match.start() - 1] if match.start() else "\n"
        if NAME_HERE.fullmatch(before):
            return True
    return False


def self_test() -> str:
    """Parse a sample whose answer is known. Each rule this script exists for
    is one line of it, so a rule quietly dropped fails here rather than
    lowering a number nobody can check."""
    parsed = {names[0]: (tag, names) for tag, names in _elements_of(SAMPLE)}
    expected = {
        "sample-field": "input",
        "sample-plain": "button",
        "sample-template": "button",
        "sample-handle": "span*",
    }
    for name, tag in expected.items():
        if name not in parsed:
            return f"the parser no longer sees {name}"
        if parsed[name][0] != tag:
            return f"{name} came back as {parsed[name][0]}, not {tag}"
    if parsed["sample-template"][1] != ("sample-template", "sample-tail"):
        return "a template class is not being read word by word around its holes"
    if "sample-box" in parsed and parsed["sample-box"][0].endswith("*"):
        return "an element with no handler is being counted as a control"
    if _nameless_of(SAMPLE) != 1:
        return "a control with no name of its own is not being counted apart"
    if touched_in('await b.click("#sample-field");', ("sample",)):
        return "a name is being matched inside another word"
    for walk in (
        'await b.click("#sample-field");',
        'document.getElementById("sample-field")',
        'b.exists(".sample-field[data-kind=\"x\"]")',
        'await b.click(`${rail} #sample-field`);',
    ):
        if not touched_in(walk, ("sample-field",)):
            return f"a name in a selector is no longer being found: {walk}"
    # Prose. A driver explains itself in English, and a control named for an
    # ordinary word - `edge`, `lane`, `section`, `in` - is otherwise pressed
    # by every comment that happens to use it.
    if touched_in("// the far edge of the rail, one sample-field at a time", ("sample-field",)):
        return "a name is being found in a sentence about the walk"
    # A shared word. `<span className="edge in">` carries `in`, and a walk
    # that presses `.fade.in` elsewhere must not vouch for the handle nobody
    # drags. Only a control's own name answers for it.
    if touched_in('await b.click(".sample-shared");', ("sample-unpressed", "sample-shared")):
        return "a name a control shares with others is vouching for it"
    return ""


def walk_sources(walks: list[Path]) -> str:
    return "\n".join(
        path.read_text()
        for directory in walks
        if directory.is_dir()
        for path in sorted(directory.glob("*.mjs")) + sorted(directory.glob("*.rs"))
    )


def main(argv: list[str]) -> int:
    pages = Path(argv[1]) if len(argv) > 1 else PAGES
    walks = [Path(directory) for directory in argv[2:]] or WALKS
    known = walk_sources(WALKS)

    # The checks, always against the Workbench's own pages and walks, so they
    # say the same thing whatever was asked for.
    wrong = self_test()
    if wrong:
        print(wrong, file=sys.stderr)
        return 2
    workbench = named_elements(PAGES)
    if not workbench or not known:
        print("nothing to read: run this from the repository", file=sys.stderr)
        return 2
    # Four controls the walks demonstrably use, one for each way of being
    # wrong: `server-command` is only ever taken by its bare name,
    # `declare-server-open` only ever as a selector, `prompt-text` both ways,
    # `server-attach` carries no id at all, and `project-row` - the button a
    # person picks a project with - is named by a template class and nothing
    # else, so dropping those loses it.
    def carrying(name: str) -> tuple[str, tuple[str, ...]] | None:
        """The element that carries this name, by any of its names."""
        for tag, names in workbench.values():
            if name in names:
                return tag, names
        return None

    for pressed in (
        "server-command",
        "declare-server-open",
        "prompt-text",
        "server-attach",
        "project-row",
    ):
        element = carrying(pressed)
        if element is None:
            print(f"the markup no longer names {pressed}, which a walk presses", file=sys.stderr)
            return 2
        if not touched_in(known, element[1]):
            print(f"the search cannot see {pressed}, which a walk presses", file=sys.stderr)
            return 2
    # And the markup is read across newlines: `package-url` opens its tag on
    # one line and is named on the next, which is how five real fields were
    # once filed as containers. Reading a line at a time loses it entirely.
    package = carrying("package-url")
    if package is None or package[0] != "input":
        print("the markup is not being read across newlines", file=sys.stderr)
        return 2

    elements = workbench if pages == PAGES else named_elements(pages)
    haystack = known if walks == WALKS else walk_sources(walks)
    if not elements:
        print(f"no named elements under {pages}", file=sys.stderr)
        return 2
    untouched = sorted(name for name, (_, names) in elements.items() if not touched_in(haystack, names))
    def acts(tag: str) -> bool:
        return tag in CONTROLS or tag.endswith("*")

    controls = [name for name in untouched if acts(elements[name][0])]
    nameless = unnameable(pages)
    all_controls = sum(1 for _, (tag, _) in elements.items() if acts(tag)) + nameless
    # The control line is the one that answers "what has nobody pressed": it
    # does not move when the parser learns to read another kind of name.
    print(
        f"{len(elements)} named elements, {len(untouched)} of them untouched. "
        f"Of {all_controls} controls, {len(controls)} are untouched and "
        f"{nameless} carry no name a walk could use."
    )
    for name in controls:
        print(f"  {elements[name][0]:9} {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
