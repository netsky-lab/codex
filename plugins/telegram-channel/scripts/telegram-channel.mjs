#!/usr/bin/env node

import readline from "node:readline";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const token = process.env.TELEGRAM_BOT_TOKEN;
const allowedChatIds = new Set(
  (process.env.TELEGRAM_ALLOWED_CHAT_IDS ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean),
);
const pollTimeoutSec = Number(process.env.TELEGRAM_POLL_TIMEOUT_SEC ?? "25");
const allowAllChats = process.env.TELEGRAM_ALLOW_ALL_CHATS === "1";
const offsetFile =
  process.env.TELEGRAM_OFFSET_FILE ??
  path.join(
    process.env.CODEX_HOME ?? path.join(os.homedir(), ".codex"),
    "telegram-channel-offset.json",
  );
const maxTelegramMessageLength = 4096;

let nextId = 1;
let initialized = false;
let polling = false;
let updateOffset = 0;
let lastChatId = null;
let acceptedUpdates = 0;
let rejectedUpdates = 0;
const recentMessages = new Map();

if (process.argv.includes("--self-test")) {
  runSelfTest()
    .then(() => process.exit(0))
    .catch((error) => {
      console.error(error.stack ?? error.message);
      process.exit(1);
    });
}

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
          version: "0.2.1",
          title: "Telegram Channel",
        },
        instructions:
          "Telegram messages arrive as JSON channel user input. Use telegram_reply with channel_message_id to answer the originating Telegram chat.",
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
                channel_message_id: {
                  type: "string",
                  description:
                    "Optional inbound channel message id. Uses that message's chat and reply id.",
                },
              },
              required: ["text"],
              additionalProperties: false,
            },
          },
          {
            name: "telegram_status",
            title: "Telegram channel status",
            description: "Report Telegram channel bridge polling and routing state.",
            inputSchema: {
              type: "object",
              properties: {},
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
  if (name === "telegram_status") {
    sendToolResult(message.id, statusText(), false);
    return;
  }
  if (name !== "telegram_reply") {
    sendError(message.id, -32602, `unknown tool: ${name}`);
    return;
  }
  if (!token) {
    sendToolResult(message.id, "TELEGRAM_BOT_TOKEN is not set.", true);
    return;
  }

  const text = String(args.text ?? "").trim();
  const route = routeForReply(args);
  const chatId = String(args.chat_id ?? route.chatId ?? lastChatId ?? "").trim();
  const replyToMessageId = args.reply_to_message_id ?? route.replyToMessageId;
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

  for (const chunk of splitTelegramMessage(text)) {
    await telegram("sendMessage", {
      chat_id: chatId,
      text: chunk,
      ...(Number.isInteger(replyToMessageId)
        ? { reply_to_message_id: replyToMessageId }
        : {}),
    });
  }
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
  if (allowedChatIds.size === 0 && !allowAllChats) {
    logError(
      "TELEGRAM_ALLOWED_CHAT_IDS is required unless TELEGRAM_ALLOW_ALL_CHATS=1; Telegram channel polling is disabled.",
    );
    return;
  }

  updateOffset = await readOffset();

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
        await writeOffset(updateOffset);
      }
    } catch (error) {
      logError(`Telegram polling failed: ${error.message}`);
      await sleep(error.retryAfterMs ?? 3000);
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
    rejectedUpdates += 1;
    return;
  }

  const text = message.text ?? message.caption;
  if (!text) {
    return;
  }

  lastChatId = chatId;
  acceptedUpdates += 1;
  const channelMessageId = `telegram:${chatId}:${message.message_id}`;
  rememberMessage(channelMessageId, {
    chatId,
    replyToMessageId: message.message_id,
  });
  sendChannelNotification("notifications/codex/channel", {
    id: channelMessageId,
    schema_version: 1,
    source: "telegram",
    channel: "telegram",
    text,
    sender: String(message.from?.id ?? chatId),
    chat_id: chatId,
    telegram_message_id: message.message_id,
    username: message.from?.username,
    first_name: message.from?.first_name,
  });
}

function chatAllowed(chatId) {
  return allowAllChats || allowedChatIds.has(String(chatId));
}

async function telegram(method, payload) {
  let attempt = 0;
  for (;;) {
    const response = await fetch(`https://api.telegram.org/bot${token}/${method}`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
      },
      body: JSON.stringify(payload),
    });
    const json = await response.json().catch(() => null);
    if (response.ok && json?.ok) {
      return json.result;
    }

    const retryAfter = json?.parameters?.retry_after;
    const retryAfterMs = Number.isFinite(retryAfter) ? retryAfter * 1000 : undefined;
    if ((response.status === 429 || response.status >= 500) && attempt < 5) {
      attempt += 1;
      await sleep(retryAfterMs ?? Math.min(30000, 1000 * 2 ** attempt));
      continue;
    }

    const error = new Error(json?.description ?? `Telegram API ${method} failed`);
    if (retryAfterMs !== undefined) {
      error.retryAfterMs = retryAfterMs;
    }
    throw error;
  }
}

function routeForReply(args) {
  const id = String(args.channel_message_id ?? "").trim();
  if (!id) {
    return {};
  }
  return recentMessages.get(id) ?? {};
}

function rememberMessage(id, route) {
  recentMessages.set(id, route);
  while (recentMessages.size > 200) {
    const oldest = recentMessages.keys().next().value;
    recentMessages.delete(oldest);
  }
}

function splitTelegramMessage(text) {
  const chunks = [];
  for (let offset = 0; offset < text.length; offset += maxTelegramMessageLength) {
    chunks.push(text.slice(offset, offset + maxTelegramMessageLength));
  }
  return chunks.length === 0 ? [""] : chunks;
}

async function readOffset() {
  try {
    const data = JSON.parse(await fs.readFile(offsetFile, "utf8"));
    return Number.isSafeInteger(data?.offset) && data.offset > 0 ? data.offset : 0;
  } catch (error) {
    if (error.code !== "ENOENT") {
      logError(`failed to read Telegram offset file: ${error.message}`);
    }
    return 0;
  }
}

async function writeOffset(offset) {
  await fs.mkdir(path.dirname(offsetFile), { recursive: true });
  const tmp = `${offsetFile}.${process.pid}.tmp`;
  await fs.writeFile(tmp, `${JSON.stringify({ offset })}\n`, { mode: 0o600 });
  await fs.rename(tmp, offsetFile);
}

function statusText() {
  return [
    `polling=${polling}`,
    "channel_delivery=logging_notification_v1",
    `allow_all_chats=${allowAllChats}`,
    `allowed_chats=${allowedChatIds.size}`,
    `offset=${updateOffset}`,
    `accepted_updates=${acceptedUpdates}`,
    `rejected_updates=${rejectedUpdates}`,
    `recent_routes=${recentMessages.size}`,
    `offset_file=${offsetFile}`,
  ].join("\n");
}

async function runSelfTest() {
  const chunks = splitTelegramMessage("x".repeat(maxTelegramMessageLength + 2));
  if (chunks.length !== 2 || chunks[0].length !== maxTelegramMessageLength || chunks[1].length !== 2) {
    throw new Error("splitTelegramMessage self-test failed");
  }
  rememberMessage("telegram:1:2", { chatId: "1", replyToMessageId: 2 });
  const route = routeForReply({ channel_message_id: "telegram:1:2" });
  if (route.chatId !== "1" || route.replyToMessageId !== 2) {
    throw new Error("routeForReply self-test failed");
  }
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

function sendChannelNotification(method, params) {
  sendNotification("notifications/message", {
    level: "info",
    logger: "codex-channel",
    data: {
      method,
      params,
    },
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
