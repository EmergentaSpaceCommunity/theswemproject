// A person comes to a Workbench served at an address.
//
// The device that holds their passkey is the browser's own: what a
// fingerprint answers is answered by an authenticator the browser is given
// for the walk. The ceremony, the page and the host are the product's.

import {launchBrowser, sleep, cleanup, browserPath} from "../../swem-host/tests/cdp_browser.mjs";

const step = (name) => console.log(`step: ${name}`);
const [url, word] = process.argv.slice(2);
const browser = browserPath();
if (!url || !word) {
  console.error("usage: door_driver.mjs <address> <word>");
  process.exit(2);
}

let b = await launchBrowser({browser, url, label: "door", door: true});

const says = (what, selector, text) =>
  b.waitFor(what, async () => b.evaluate(
    `[...document.querySelectorAll(${JSON.stringify(selector)})].some((one) => one.innerText.includes(${JSON.stringify(text)}))`), 200);
const press = async (text, among = "button, a.k-btn") => {
  const at = JSON.parse(await b.evaluate(`(() => {
    const found = [...document.querySelectorAll(${JSON.stringify(among)})]
      .find((one) => one.innerText.trim().includes(${JSON.stringify(text)}) && !one.disabled);
    if (!found) return "null";
    found.scrollIntoView({block: "center"});
    const box = found.getBoundingClientRect();
    return JSON.stringify({x: box.left + box.width / 2, y: box.top + box.height / 2});
  })()`));
  if (!at) throw new Error(`nothing a person could press says "${text}"`);
  await b.clickAt(at.x, at.y);
  await sleep(300);
};
const write = async (label, text) => {
  const found = await b.evaluate(`(() => {
    const field = [...document.querySelectorAll("label")]
      .find((one) => one.innerText.includes(${JSON.stringify(label)}))?.querySelector("input");
    if (!field) return false;
    field.focus();
    field.select();
    return true;
  })()`);
  if (!found) throw new Error(`no field is called "${label}"`);
  await b.send("Input.insertText", {text});
  await sleep(150);
};
const saidOnce = () => b.evaluate(`document.querySelector('[role="dialog"] .w-said-once')?.innerText.trim() ?? ""`);
const inside = () => says("the Workbench", 'nav[aria-label="Workbench"]', "Settings");
const toAccess = async () => {
  await press("Settings", 'nav[aria-label="Workbench"] button');
  await says("who may come in", "h2", "Devices that may come in");
};

step("the first start asks for the word");
await b.holdPasskeys();
// A served host that belongs to nobody offers the choice first: this
// device is mine, or add it to the hosts one has.
await says("the choice at the door", "h1", "Make it yours");
await press("This device is mine");
await says("the first start", "h1", "Make this Workbench yours");
// Typed as a person types what they read off a terminal.
await write("The word it printed", word.toUpperCase().replaceAll("-", " "));
await write("A name for this device", "Laptop");
await press("Register this device");

step("the codes are shown once");
await says("the codes", "h1", "Codes to come back with");
const codes = JSON.parse(await b.evaluate(
  `JSON.stringify([...document.querySelectorAll(".w-code")].map((one) => one.innerText.trim()))`));
if (codes.length !== 8) throw new Error(`eight codes are made; the page shows ${codes.length}`);
await b.evaluate(`document.querySelector('input[type="checkbox"]').click()`);
await press("Open the Workbench");
await inside();

step("a token is made for a program");
await toAccess();
await says("this device", ".k-chip", "This device");
await press("Make a token");
await write("What it is for", "Morning scheduler");
await press("Make it");
await says("the token, once", '[role="dialog"]', "swem_");
const token = await saidOnce();
await press("Done");
await says("the token in the list", ".k-name", "Morning scheduler");
const shownAgain = await b.evaluate(`document.body.innerText.includes(${JSON.stringify(token)})`);
if (shownAgain) throw new Error("the token is shown again after it was closed");
console.log(`token: ${token}`);

step("signed out, nothing behind the door answers");
await press("Sign out");
await says("sign in", "h1", "Sign in");
const behind = await b.evaluate(`fetch("/api/profiles").then((answer) => answer.status)`);
if (behind !== 401) throw new Error(`signed out, the profiles answered ${behind}`);

step("the passkey lets them in");
await press("Sign in with a passkey");
await inside();
await b.close();

step("a browser that holds nothing is refused");
b = await launchBrowser({browser, url, label: "door-another", door: true});
await b.holdPasskeys();
await says("sign in", "h1", "Sign in");
await press("Sign in with a passkey");
await says("the refusal", '[role="alert"]', "Nobody came in");

step("a code brings them back, to register this device");
await press("Every device is lost");
await write("One of the codes", codes[0]);
await press("Come back");
await says("registering", "h1", "Register this device");
const withACode = await b.evaluate(`fetch("/api/profiles").then((answer) => answer.status)`);
if (withACode !== 403) throw new Error(`with a code and no device, the profiles answered ${withACode}`);
await write("A name for this device", "Phone");
await press("Register this device");
await inside();
await toAccess();
await says("both devices", ".k-name", "Laptop");
await says("both devices", ".k-name", "Phone");

step("the first device is taken away");
await press("Take away");
await press("Take it away");
await b.waitFor("one device is left", async () => b.evaluate(
  `![...document.querySelectorAll(".k-name")].some((one) => one.innerText === "Laptop")`), 100);
await says("what was done", ".w-happened", "A device was taken away: Laptop");

console.log("door OK");
cleanup(0, "door OK");
