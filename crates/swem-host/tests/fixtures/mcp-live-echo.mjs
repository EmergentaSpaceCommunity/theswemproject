import fs from "node:fs";
import readline from "node:readline";

const receiptFlag = process.argv.indexOf("--receipt");
if (receiptFlag < 0 || !process.argv[receiptFlag + 1]) {
  throw new Error("--receipt is required");
}
const receiptPath = process.argv[receiptFlag + 1];
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

for await (const line of lines) {
  if (!line.trim()) continue;
  const request = JSON.parse(line);
  if (request.id === undefined) continue;

  if (request.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: request.id,
      result: {
        protocolVersion: request.params?.protocolVersion ?? "2025-06-18",
        capabilities: { tools: {} },
        serverInfo: { name: "swem-live-echo", version: "0.1.0" },
      },
    });
  } else if (request.method === "tools/list") {
    send({
      jsonrpc: "2.0",
      id: request.id,
      result: {
        tools: [
          {
            name: "echo",
            description: "Record and return an exact caller-provided nonce.",
            inputSchema: {
              type: "object",
              properties: { nonce: { type: "string" } },
              required: ["nonce"],
              additionalProperties: false,
            },
          },
        ],
      },
    });
  } else if (request.method === "tools/call" && request.params?.name === "echo") {
    const nonce = request.params?.arguments?.nonce;
    if (typeof nonce !== "string") {
      send({
        jsonrpc: "2.0",
        id: request.id,
        result: { content: [{ type: "text", text: "nonce must be a string" }], isError: true },
      });
      continue;
    }
    const receipt = { nonce, server: "swem-live-echo" };
    fs.writeFileSync(receiptPath, `${JSON.stringify(receipt)}\n`, { encoding: "utf8", mode: 0o600 });
    send({
      jsonrpc: "2.0",
      id: request.id,
      result: {
        content: [{ type: "text", text: JSON.stringify(receipt) }],
        structuredContent: receipt,
      },
    });
  } else {
    send({
      jsonrpc: "2.0",
      id: request.id,
      error: { code: -32601, message: `Unsupported method ${request.method}` },
    });
  }
}
