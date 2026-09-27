// What an App says a person is looking at is given to the agent with the
// next turn: a session that attaches the server, the server's space, the
// App's own control, and the Agent space showing what the agent is given.
// usage: node workbench_shell_context_cdp_driver.mjs <browser> <url>
import {launchBrowser, cleanup, sleep} from "./cdp_browser.mjs";

const [browser, url] = process.argv.slice(2);
if (!browser || !url) {
  console.error("usage: node workbench_shell_context_cdp_driver.mjs <browser> <url>");
  process.exit(2);
}
const b = await launchBrowser({browser, url, label: "context"});
const step = (s) => console.log("step: " + s);
const inApp = (expression) => b.evaluateEverywhere(expression);
const pressInApp = async (id) => {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    const pressed = await inApp(`(() => {
      const control = document.getElementById(${JSON.stringify(id)});
      if (!control || control.disabled) return null;
      control.click();
      return "pressed";
    })()`);
    if (pressed === "pressed") return;
    await sleep(200);
  }
  cleanup(2, `the App offers no ${id}`);
};
const appSays = () => inApp(`document.getElementById("note-status")?.textContent || null`);
const given = () => b.dataset("session-context");

// 1. No Project space of the host's own: Agent, Store, and the server's.
await b.waitFor("the shell", async () => b.exists("#space-store"));
if (await b.exists("#space-project")) cleanup(2, "the host still offers a Project space of its own");
await b.waitFor("the server's space on the switcher", async () => b.exists("#space-server-notes"), 300);
step("the switcher offers Agent, Store and the server's space");

// 2. A session whose profile attaches the server.
await b.waitFor("profiles loaded", async () =>
  (await b.evaluate(`document.getElementById("profiles").options.length`)) >= 1);
await b.click("#open-new");
await b.waitFor("a session", async () => (await b.textOf("connection-id")) !== "-", 300);
const route = await b.textOf("route");
if ((await given()).serverName) cleanup(2, "a fresh session is given something nobody said");
step("a session is open and is given nothing yet");

// 3. In the server's space, the App says what a person is looking at.
await b.click("#space-server-notes");
await b.waitFor("the App mounted", async () => b.evaluate(`!!document.querySelector('#server-space .domain-app iframe')`), 300);
await pressInApp("give-context");
await b.waitFor("the App heard the host take it", async () => (await appSays()) === "context given", 100);
step("the App said what the person is looking at, and the host took it");

// 4. The Agent space shows what the agent is given, and from which server.
await b.click("#space-agent");
await b.waitFor("what the agent is given", async () => (await given()).serverName === "notes", 100);
const blocks = await b.evaluate(
  `JSON.stringify([...document.querySelectorAll("#session-context .context-block")].map((row) => [row.dataset.kind, row.textContent]))`);
step(`the agent is given ${blocks}`);

// 5. A turn carries it.
await b.evaluate(`document.getElementById("prompt-text").value = ""`);
await b.click("#prompt-text");
await b.send("Input.insertText", {text: "what am I looking at?"});
await b.waitFor("the ask is typed", async () =>
  (await b.evaluate(`document.getElementById("prompt-text").value`)).includes("looking at"));
await b.click("#send");
await b.waitFor("the turn ends", async () => (await b.textOf("turn-outcome")).startsWith("turn:"), 300);
step("a turn went with it");

// 6. Let go of it, from the Agent space.
await b.click("#session-context-clear");
await b.waitFor("nothing is given any more", async () => !(await given()).serverName, 100);
step("let go of");

console.log(JSON.stringify({route, blocks: JSON.parse(blocks)}));
cleanup(0);
