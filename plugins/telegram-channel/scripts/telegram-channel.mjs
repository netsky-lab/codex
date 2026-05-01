#!/usr/bin/env node

import readline from "node:readline";

const token = process.env.TELEGRAM_BOT_TOKEN;
const allowedChatIds = new Set(
  (process.env.TELEGRAM_ALLOWED_CHAT_IDS ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean),
);
const pollTimeoutSec = Number(process.env.TELEGRAM_POLL_TIMEOUT_SEC ?? "25");

let nextId = 1;
let initialized = false;
let polling = false;
let updateOffset = 0;
let lastChatId = null;

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

rl.on("line", async (line) => {
  if (!line.trim()) {
    return;
  }

  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    logError(`invalid JSON-RPC payload: ${error.message}`);
    return;
  }

  try {
    if (message.id !== undefined) {
      await handleRequest(message);
    } else if (message.method === "notifications/initialized" || message.method === "initialized") {
      initialized = true;
      startPolling();
    }
  } catch (error) {
    if (message.id !== undefined) {
      sendError(message.id, -32603, error.message);
    } else {
      logError(error.message);
    }
  }
});

process.on("SIGINT", () => process.exit(0));
process.on("SIGTERM", () => process.exit(0));

async function handleRequest(message) {
  switch (message.method) {
    case "initialize":
      sendResult(message.id, {
        protocolVersion: "2025-06-18",
        capabilities: {
          tools: {},
        },
        serverInfo: {
          name: "telegram-channel",
          version: "0.1.0",
          title: "Telegram Channel",
        },
        instructions:
          "Telegram messages arrive as <channel source=\"telegram\"> user input. Use telegram_reply to answer in Telegram.",
      });
      break;
    case "ping":
      sendResult(message.id, {});
      break;
    case "tools/list":
      sendResult(message.id, {
        tools: [
          {
            name: "telegram_reply",
            title: "Reply in Telegram",
            description:
              "Send a message back to the Telegram chat that produced the latest channel message, or to a specific chat_id.",
            inputSchema: {
              type: "object",
              properties: {
                text: {
                  type: "string",
                  description: "Message text to send.",
                },
                chat_id: {
                  type: "string",
                  description:
                    "Optional Telegram chat id. Defaults to the most recent allowed inbound chat.",
                },
                reply_to_message_id: {
                  type: "integer",
                  description: "Optional Telegram message id to reply to.",
                },
              },
              required: ["text"],
              additionalProperties: false,
            },
          },
        ],
      });
      break;
    case "tools/call":
      await handleToolCall(message);
      break;
    default:
      sendError(message.id, -32601, `unsupported method: ${message.method}`);
      break;
  }
}

async function handleToolCall(message) {
  const { name, arguments: args = {} } = message.params ?? {};
  if (name !== "telegram_reply") {
    sendError(message.id, -32602, `unknown tool: ${name}`);
    return;
  }
  if (!token) {
    sendToolResult(message.id, "TELEGRAM_BOT_TOKEN is not set.", true);
    return;
  }

  const text = String(args.text ?? "").trim();
  const chatId = String(args.chat_id ?? lastChatId ?? "").trim();
  const replyToMessageId = args.reply_to_message_id;
  if (!text) {
    sendToolResult(message.id, "text is required.", true);
    return;
  }
  if (!chatId) {
    sendToolResult(message.id, "No Telegram chat is available yet.", true);
    return;
  }
  if (!chatAllowed(chatId)) {
    sendToolResult(message.id, `Telegram chat ${chatId} is not allowed.`, true);
    return;
  }

  await telegram("sendMessage", {
    chat_id: chatId,
    text,
    parse_mode: "Markdown",
    ...(Number.isInteger(replyToMessageId)
      ? { reply_to_message_id: replyToMessageId }
      : {}),
  });
  sendToolResult(message.id, `sent to Telegram chat ${chatId}`, false);
}

function sendToolResult(id, text, isError) {
  sendResult(id, {
    content: [
      {
        type: "text",
        text,
      },
    ],
    isError,
  });
}

function startPolling() {
  if (polling || !initialized) {
    return;
  }
  polling = true;
  void pollLoop();
}

async function pollLoop() {
  if (!token) {
    logError("TELEGRAM_BOT_TOKEN is not set; Telegram channel polling is disabled.");
    return;
  }

  for (;;) {
    try {
      const result = await telegram("getUpdates", {
        offset: updateOffset,
        timeout: pollTimeoutSec,
        allowed_updates: ["message"],
      });
      for (const update of result) {
        updateOffset = Math.max(updateOffset, update.update_id + 1);
        await handleTelegramUpdate(update);
      }
    } catch (error) {
      logError(`Telegram polling failed: ${error.message}`);
      await sleep(3000);
    }
  }
}

async function handleTelegramUpdate(update) {
  const message = update.message;
  if (!message?.chat?.id) {
    return;
  }

  const chatId = String(message.chat.id);
  if (!chatAllowed(chatId)) {
    return;
  }

  const text = message.text ?? message.caption;
  if (!text) {
    return;
  }

  lastChatId = chatId;
  sendNotification("notifications/codex/channel", {
    source: "telegram",
    channel: "telegram",
    text,
    sender: String(message.from?.id ?? chatId),
    chat_id: chatId,
    message_id: message.message_id,
    username: message.from?.username,
    first_name: message.from?.first_name,
  });
}

function chatAllowed(chatId) {
  return allowedChatIds.size === 0 || allowedChatIds.has(String(chatId));
}

async function telegram(method, payload) {
  const response = await fetch(`https://api.telegram.org/bot${token}/${method}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(payload),
  });
  const json = await response.json().catch(() => null);
  if (!response.ok || !json?.ok) {
    throw new Error(json?.description ?? `Telegram API ${method} failed`);
  }
  return json.result;
}

function sendResult(id, result) {
  write({
    jsonrpc: "2.0",
    id,
    result,
  });
}

function sendError(id, code, message) {
  write({
    jsonrpc: "2.0",
    id,
    error: {
      code,
      message,
    },
  });
}

function sendNotification(method, params) {
  write({
    jsonrpc: "2.0",
    method,
    params,
  });
}

function logError(message) {
  sendNotification("notifications/message", {
    level: "error",
    logger: "telegram-channel",
    data: message,
  });
}

function write(payload) {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
