import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { getMigrations } from "better-auth/db/migration";
import { CLIENT_ID, createAuth, REDIRECT_URI, SCOPES, type Auth } from "./auth";
import { PWNED_PASSWORDS_URL, readConfig, type Config } from "./config";
import { createMailer } from "./mail";
import { createSyncTable } from "./sync";

type Server = { auth: Auth; config: Config };

const state = globalThis as typeof globalThis & { nuzkyAccount?: Promise<Server> };

export function getServer(): Promise<Server> {
  state.nuzkyAccount ??= start();
  return state.nuzkyAccount;
}

export async function startOrExit() {
  try {
    await getServer();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

export async function getAuth(): Promise<Auth> {
  return (await getServer()).auth;
}

async function start(): Promise<Server> {
  const config = readConfig();
  mkdirSync(path.dirname(config.databasePath), { recursive: true });
  const database = new DatabaseSync(config.databasePath);
  database.exec("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;");
  if (config.pwnedPasswordsUrl !== PWNED_PASSWORDS_URL) sendPwnedPasswordChecksTo(config.pwnedPasswordsUrl);

  const auth = createAuth(config, database, createMailer(config.mail));
  const { runMigrations } = await getMigrations(auth.options);
  await runMigrations();
  createSyncTable(database);
  await seedDesktopClient(auth);
  await deleteUnverifiedAccounts(auth, config.unverifiedTtlSeconds);
  const sweep = setInterval(
    () =>
      deleteUnverifiedAccounts(auth, config.unverifiedTtlSeconds).catch((error) =>
        console.error("Deleting unverified accounts failed", error),
      ),
    Math.min(config.unverifiedTtlSeconds, 60 * 60) * 1000,
  );
  sweep.unref();
  return { auth, config };
}

async function seedDesktopClient(auth: Auth) {
  const { adapter } = await auth.$context;
  const client = {
    clientId: CLIENT_ID,
    name: "Nuzky",
    disabled: false,
    redirectUris: [REDIRECT_URI],
    tokenEndpointAuthMethod: "none",
    grantTypes: ["authorization_code", "refresh_token"],
    responseTypes: ["code"],
    applicationType: "native",
    requirePKCE: true,
    skipConsent: true,
    scopes: SCOPES,
    updatedAt: new Date(),
  };
  const where = [{ field: "clientId", value: CLIENT_ID }];
  const existing = await adapter.findOne({ model: "oauthClient", where });
  if (existing) await adapter.update({ model: "oauthClient", where, update: client });
  else await adapter.create({ model: "oauthClient", data: { ...client, createdAt: new Date() } });
}

async function deleteUnverifiedAccounts(auth: Auth, ttlSeconds: number) {
  const { adapter, internalAdapter } = await auth.$context;
  const stale = await adapter.findMany<{ id: string }>({
    model: "user",
    where: [
      { field: "emailVerified", value: false },
      { field: "createdAt", operator: "lt", value: new Date(Date.now() - ttlSeconds * 1000) },
    ],
  });
  for (const { id } of stale) {
    const accounts = await internalAdapter.findAccounts(id);
    if (accounts.every((account) => account.providerId === "credential")) await internalAdapter.deleteUser(id);
  }
}

function sendPwnedPasswordChecksTo(url: string) {
  const fetchFromInternet = globalThis.fetch;
  globalThis.fetch = (input, init) => {
    const target = input instanceof Request ? input.url : String(input);
    if (!target.startsWith(`${PWNED_PASSWORDS_URL}/`)) return fetchFromInternet(input, init);
    return fetchFromInternet(url + target.slice(PWNED_PASSWORDS_URL.length), init);
  };
}
