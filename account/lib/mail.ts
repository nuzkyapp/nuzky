import { randomUUID } from "node:crypto";
import { mkdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import nodemailer from "nodemailer";
import type { Config } from "./config";

export type Message = { to: string; subject: string; text: string; code: string | null };

export type Mailer = (message: Message) => Promise<void>;

export function createMailer(config: Config["mail"]): Mailer {
  if (config.transport === "file") {
    return async (message) => {
      await mkdir(config.dir, { recursive: true });
      const name = `${Date.now()}-${randomUUID()}.json`;
      const partial = path.join(config.dir, `.${name}.part`);
      await writeFile(partial, JSON.stringify(message), { mode: 0o600 });
      await rename(partial, path.join(config.dir, name));
    };
  }
  const transport = nodemailer.createTransport(config.url);
  return async ({ to, subject, text }) => {
    await transport.sendMail({ from: config.from, to, subject, text });
  };
}

export function codeMessage(to: string, code: string, purpose: "verify" | "reset"): Message {
  if (purpose === "verify") {
    return {
      to,
      code,
      subject: `${code} is your Nuzky code`,
      text: `Enter ${code} on the page where you are creating your Nuzky account. The code works for 10 minutes.\n\nIf you didn't try to create a Nuzky account, you can ignore this email.\n`,
    };
  }
  return {
    to,
    code,
    subject: `${code} is your code to reset your Nuzky password`,
    text: `Enter ${code} on the page where you are resetting your Nuzky password. The code works for 10 minutes.\n\nIf you didn't ask for it, you can ignore this email. Your password stays the same.\n`,
  };
}

export function existingAccountMessage(to: string, baseURL: string): Message {
  return {
    to,
    code: null,
    subject: "You already have a Nuzky account",
    text: `Someone, probably you, tried to create a Nuzky account with this address. It already has one.\n\nSign in with your password or with Google. If you don't remember the password, choose Forgot password at ${baseURL}/forgot-password to set a new one.\n\nIf it wasn't you, you can ignore this email.\n`,
  };
}
