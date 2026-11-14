import path from "node:path";

export const GOOGLE_DISCOVERY_URL = "https://accounts.google.com/.well-known/openid-configuration";
export const PWNED_PASSWORDS_URL = "https://api.pwnedpasswords.com";

export type Config = {
  mode: "production" | "test";
  baseURL: string;
  secret: string;
  databasePath: string;
  google: { clientId: string; clientSecret: string; discoveryUrl: string };
  mail: { transport: "smtp"; url: string; from: string } | { transport: "file"; dir: string; from: string };
  pwnedPasswordsUrl: string;
  refreshReuseSeconds: number;
  unverifiedTtlSeconds: number;
};

function required(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]?.trim();
  if (!value) throw new Error(`${name} is not set.`);
  return value;
}

function seconds(env: NodeJS.ProcessEnv, name: string, fallback: number): number {
  const raw = env[name];
  if (raw === undefined || raw === "") return fallback;
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${name} must be a whole number of seconds.`);
  return value;
}

export function readConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const mode = env.ACCOUNT_MODE === "test" ? "test" : "production";
  const baseURL = required(env, "BASE_URL").replace(/\/+$/, "");
  const secret = required(env, "AUTH_SECRET");
  const dataDir = env.DATA_DIR?.trim() || "/data";
  const transport = env.MAIL_TRANSPORT?.trim() || "smtp";
  const from = env.MAIL_FROM?.trim() || "Nuzky <account@nuzky.app>";

  const mail: Config["mail"] =
    transport === "file"
      ? { transport: "file", dir: required(env, "MAIL_DIR"), from }
      : transport === "smtp"
        ? { transport: "smtp", url: required(env, "SMTP_URL"), from }
        : (() => {
            throw new Error(`MAIL_TRANSPORT must be smtp or file, not ${transport}.`);
          })();

  const config: Config = {
    mode,
    baseURL,
    secret,
    databasePath: path.join(dataDir, "account.db"),
    google: {
      clientId: required(env, "GOOGLE_CLIENT_ID"),
      clientSecret: required(env, "GOOGLE_CLIENT_SECRET"),
      discoveryUrl: env.GOOGLE_DISCOVERY_URL?.trim() || GOOGLE_DISCOVERY_URL,
    },
    mail,
    pwnedPasswordsUrl: PWNED_PASSWORDS_URL,
    refreshReuseSeconds: 60,
    unverifiedTtlSeconds: 24 * 60 * 60,
  };

  if (mode === "test") {
    config.pwnedPasswordsUrl = env.PWNED_PASSWORDS_URL?.trim() || PWNED_PASSWORDS_URL;
    config.refreshReuseSeconds = seconds(env, "REFRESH_REUSE_SECONDS", config.refreshReuseSeconds);
    config.unverifiedTtlSeconds = seconds(env, "UNVERIFIED_TTL_SECONDS", config.unverifiedTtlSeconds);
    return config;
  }

  const problems: string[] = [];
  if (!baseURL.startsWith("https://")) problems.push("BASE_URL must start with https://");
  if (secret.length < 32) problems.push("AUTH_SECRET must be at least 32 characters");
  if (config.google.discoveryUrl !== GOOGLE_DISCOVERY_URL) problems.push(`GOOGLE_DISCOVERY_URL must be ${GOOGLE_DISCOVERY_URL}`);
  if (mail.transport !== "smtp") problems.push("MAIL_TRANSPORT must be smtp");
  for (const name of ["PWNED_PASSWORDS_URL", "REFRESH_REUSE_SECONDS", "UNVERIFIED_TTL_SECONDS"]) {
    if (env[name]) problems.push(`${name} is only for ACCOUNT_MODE=test`);
  }
  if (problems.length) throw new Error(`Refusing to start in production: ${problems.join("; ")}.`);
  return config;
}
