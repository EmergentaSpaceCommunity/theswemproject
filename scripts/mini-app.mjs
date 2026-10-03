// The page a bot opens inside the messenger, as one file to host anywhere:
// the palette and kit inlined, no channel written in (the bot's button says
// which, and where the Workbench is).
//
// Usage, from the repository root:
//   node scripts/mini-app.mjs      # writes web/mini-app/dist/index.html
import fs from "node:fs";
import path from "node:path";

const root = path.dirname(new URL(import.meta.url).pathname);
const read = (file) => fs.readFileSync(path.join(root, "..", file), "utf8");
const page = read("web/mini-app/index.html")
  .replace("__PALETTE_CSS__", read("web/view-kit/palette.css"))
  .replace("__KIT_CSS__", read("web/view-kit/kit.css"))
  .replace("__CHANNEL_ID__", "");
fs.mkdirSync(path.join(root, "..", "web/mini-app/dist"), {recursive: true});
fs.writeFileSync(path.join(root, "..", "web/mini-app/dist/index.html"), page);
console.log("web/mini-app/dist/index.html");
