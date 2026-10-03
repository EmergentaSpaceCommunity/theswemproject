// What the messenger walks share: the messenger's side of things (a Bot API
// that is a fixture, with a side door), and adding a bot on the page.

import {cleanup} from "../../swem-host/tests/cdp_browser.mjs";

/// What the walks give the channel as the bot's token. Not a token of anything.
export const TOKEN = "123456:walk-not-a-token-of-anything";
export const OWNER = {id: 7, is_bot: false, first_name: "Ada", username: "ada"};
export const STRANGER = {id: 9, is_bot: false, first_name: "Bob", username: "bob"};

/// The messenger's side, as the fixture lets a walk play it.
export function messenger(api) {
  let messageId = 0;
  const post = async (update) => {
    const response = await fetch(`${api}/_fixture/updates`, {
      method: "POST",
      headers: {"content-type": "application/json"},
      body: JSON.stringify(update),
    });
    if (!response.ok) cleanup(1, `the fixture refused an update: ${response.status}`);
  };
  /// Somebody writes to the bot, alone or in a chat; `extra` goes on the
  /// message (a document, a topic).
  const written = async (from, text, {chat, extra = {}} = {}) => {
    messageId += 1;
    await post({
      message: {
        message_id: messageId,
        from,
        chat: chat ?? {id: from.id, type: "private", first_name: from.first_name},
        date: 0,
        ...(text === undefined ? {} : {text}),
        ...extra,
      },
    });
  };
  /// Somebody presses a button under a message.
  const pressed = async (from, chat, data) => {
    messageId += 1;
    await post({callback_query: {id: `cq${messageId}`, from, message: {message_id: messageId, chat}, data}});
  };
  /// What the bot sent, as the messenger recorded it: `{method, body}` in order.
  const sent = async () => (await (await fetch(`${api}/_fixture/sent`)).json()).result ?? [];
  const sentTo = (calls, chat, method = "sendMessage") =>
    calls.filter((call) => call.method === method && String(call.body?.chat_id) === String(chat));
  return {written, pressed, sent, sentTo};
}

/// Add a bot on Providers → Channels and pair the owner with the code.
/// Returns the pairing code.
export async function addBotAndPair(b, api, profile, {guests = "nobody", tg}) {
  const choose = async (selector, value) =>
    b.evaluate(`(() => {
      const field = document.querySelector(${JSON.stringify(selector)});
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(field, ${JSON.stringify(value)});
      field.dispatchEvent(new Event('change', {bubbles: true}));
    })()`);
  const options = (selector) => b.evaluate(`[...(document.querySelector(${JSON.stringify(selector)})?.options ?? [])].map((o) => o.value)`);
  await b.goTo("#/providers/channels");
  await b.waitFor("the channels page offers a bot", async () => b.exists("#channel-add-bot"), 100);
  await b.click("#channel-add-bot");
  await b.waitFor("the form for a bot", async () => b.exists("#channel-name"));
  if (!(await options("#channel-package")).includes("telegram")) cleanup(1, `the product came with no Telegram channel: ${JSON.stringify(await options("#channel-package"))}`);
  await b.fill("#channel-name", "My bot");
  await choose("#channel-package", "telegram");
  await b.fill("#channel-key", TOKEN);
  if (!(await options("#channel-agent")).includes(profile)) cleanup(1, `the agent cannot be chosen to answer: ${JSON.stringify(await options("#channel-agent"))}`);
  await choose("#channel-agent", profile);
  await choose("#channel-guests", guests);
  await b.click("#channel-more");
  await b.waitFor("the address field", async () => b.exists("#channel-api-root"));
  await b.fill("#channel-api-root", api);
  await b.click("#channel-add");
  await b.waitFor("the bot is running", async () => b.exists('.channel-row[data-running="true"]'), 150);
  const row = await b.evaluate(`document.querySelector('.channel-row')?.innerText ?? ""`);
  if (!row.includes("@swem_fixture_bot")) cleanup(1, `the bot was not looked at: ${row}`);
  if ((await b.evaluate("document.body.innerText")).includes(TOKEN)) cleanup(1, "the page shows the token back");
  const code = (await b.evaluate(`document.querySelector('.channel-code')?.textContent ?? ""`)).trim();
  if (!/^\d{6}$/.test(code)) cleanup(1, `no code to say to the bot: ${JSON.stringify(code)}`);
  await tg.written(OWNER, code);
  await b.waitFor("the page shows the person paired", async () => b.exists('.channel-row[data-paired="true"]'), 150);
  const welcome = tg.sentTo(await tg.sent(), OWNER.id);
  if (!welcome.some((call) => String(call.body.text).includes("known here"))) cleanup(1, `the bot did not say the person is known: ${JSON.stringify(welcome)}`);
  return code;
}
