import { randomUUID } from "node:crypto";
import { mkdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import nodemailer from "nodemailer";
import type { Config } from "./config";

export type Message = { to: string; subject: string; text: string; html: string; code: string | null };

export type Mailer = (message: Message) => Promise<void>;

const LOGO_CID = "logo@nuzky.app";
const LOGO_PNG = "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";
const SANS = "Inter,-apple-system,'SF Pro Text','Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const MONO = "'SF Mono',ui-monospace,Menlo,Consolas,monospace";

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
  return async ({ to, subject, text, html }) => {
    const logo = { filename: "nuzky.png", content: LOGO_PNG, encoding: "base64", cid: LOGO_CID };
    await transport.sendMail({ from: config.from, to, subject, text, html, attachments: [logo] });
  };
}

function page(subject: string, title: string, body: string, foot: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">
<meta name="color-scheme" content="light">
<meta name="supported-color-schemes" content="light">
<title>${subject}</title>
</head>
<body style="margin:0;padding:0;background:#f2f2f5;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f2f2f5;">
<tr><td align="center" style="padding:24px 12px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:480px;">
<tr><td style="background:#060607;border-radius:12px 12px 0 0;padding:18px 28px;">
<img src="cid:${LOGO_CID}" width="24" height="24" alt="" style="display:inline-block;vertical-align:middle;border:0;">
<span style="vertical-align:middle;padding-left:6px;font:600 15px ${SANS};color:#f2f2f5;">Nuzky</span>
</td></tr>
<tr><td style="background:#ffffff;border-radius:0 0 12px 12px;padding:30px 28px 26px;font:14px/1.55 ${SANS};color:#424245;">
<h1 style="margin:0 0 10px;font:600 21px/1.3 ${SANS};color:#1d1d1f;">${title}</h1>
${body}
</td></tr>
<tr><td style="padding:18px 28px 0;text-align:center;font:12px/1.5 ${SANS};color:#86868b;">${foot}</td></tr>
</table>
</td></tr>
</table>
</body>
</html>
`;
}

function paragraph(text: string): string {
  return `<p style="margin:0 0 14px;">${text}</p>`;
}

function digits(code: string): string {
  const box = `display:inline-block;width:36px;margin:0 3px;border:1px solid #d2d2d7;border-radius:10px;line-height:48px;font:600 24px/48px ${MONO};color:#1d1d1f;`;
  return `<p style="margin:24px 0;text-align:center;">${[...code].map((digit) => `<span style="${box}">${digit}</span>`).join("")}</p>`;
}

export function codeMessage(to: string, code: string, purpose: "verify" | "reset"): Message {
  if (purpose === "verify") {
    const subject = `${code} is your Nuzky code`;
    return {
      to,
      code,
      subject,
      text: `Enter ${code} on the page where you are creating your Nuzky account. The code works for 10 minutes.\n\nIf you didn't try to create a Nuzky account, you can ignore this email.\n`,
      html: page(
        subject,
        "Your code",
        paragraph("Enter it on the page where you are creating your Nuzky account.") + digits(code) + paragraph("It works for 10 minutes."),
        "If you didn't try to create a Nuzky account, you can ignore this email.",
      ),
    };
  }
  const subject = `${code} is your code to reset your Nuzky password`;
  return {
    to,
    code,
    subject,
    text: `Enter ${code} on the page where you are resetting your Nuzky password. The code works for 10 minutes.\n\nIf you didn't ask for it, you can ignore this email. Your password stays the same.\n`,
    html: page(
      subject,
      "Your code",
      paragraph("Enter it on the page where you are resetting your Nuzky password.") + digits(code) + paragraph("It works for 10 minutes."),
      "If you didn't ask for it, you can ignore this email. Your password stays the same.",
    ),
  };
}

export function existingAccountMessage(to: string, baseURL: string): Message {
  const subject = "You already have a Nuzky account";
  const button = `<a href="${baseURL}/forgot-password" style="display:inline-block;margin:6px 0 4px;padding:10px 18px;border-radius:8px;background:#0071e3;color:#ffffff;text-decoration:none;font:600 14px ${SANS};">Set a new password</a>`;
  return {
    to,
    code: null,
    subject,
    text: `Someone, probably you, tried to create a Nuzky account with this address. It already has one.\n\nSign in with your password or with Google. If you don't remember the password, choose Forgot password at ${baseURL}/forgot-password to set a new one.\n\nIf it wasn't you, you can ignore this email.\n`,
    html: page(
      subject,
      "You already have an account",
      paragraph("Someone, probably you, tried to create a Nuzky account with this address. Sign in with your password or with Google.") + button,
      "If it wasn't you, you can ignore this email.",
    ),
  };
}
