import type { DatabaseSync } from "node:sqlite";
import { oauthProvider } from "@better-auth/oauth-provider";
import { hash, verify, type Options as ArgonOptions } from "@node-rs/argon2";
import { betterAuth } from "better-auth";
import { emailOTP, genericOAuth, haveIBeenPwned, jwt } from "better-auth/plugins";
import type { Config } from "./config";
import { FRESH_SIGN_IN_SECONDS } from "./fresh";
import { codeMessage, existingAccountMessage, type Mailer } from "./mail";
import { sync } from "./sync-endpoints";

export const CLIENT_ID = "nuzky-desktop";
export const SCOPES = ["openid", "email", "profile", "offline_access", "nuzky"];
export const REDIRECT_URI = "http://127.0.0.1/auth/callback";
const LOOPBACK_CALLBACK = /^http:\/\/127\.0\.0\.1:([1-9]\d{0,4})\/auth\/callback$/;
const AUTHORIZATION_CODE_SECONDS = 120;
const SIGN_IN_PAGE_SECONDS = 15 * 60;

const ARGON2ID: ArgonOptions = { algorithm: 2, memoryCost: 19456, timeCost: 2, parallelism: 1 };

export function isLoopbackCallback(uri: string): boolean {
  const match = LOOPBACK_CALLBACK.exec(uri);
  return match !== null && Number(match[1]) <= 65535;
}

function isAuthorizationCode(value: string): boolean {
  try {
    return JSON.parse(value)?.type === "authorization_code";
  } catch {
    return false;
  }
}

export function createAuth(config: Config, database: DatabaseSync, mail: Mailer) {
  return betterAuth({
    appName: "Nuzky",
    baseURL: config.baseURL,
    basePath: "/api/auth",
    secret: config.secret,
    database,
    telemetry: { enabled: false },
    emailAndPassword: {
      enabled: true,
      requireEmailVerification: true,
      minPasswordLength: 10,
      maxPasswordLength: 128,
      revokeSessionsOnPasswordReset: true,
      password: {
        hash: (password) => hash(password, ARGON2ID),
        verify: ({ hash: stored, password }) => verify(stored, password),
      },
      onExistingUserSignUp: ({ user }) => mail(existingAccountMessage(user.email, config.baseURL)),
    },
    emailVerification: { sendOnSignUp: true, sendOnSignIn: true, autoSignInAfterVerification: true },
    session: { freshAge: FRESH_SIGN_IN_SECONDS },
    user: { deleteUser: { enabled: true } },
    account: {
      encryptOAuthTokens: true,
      accountLinking: { enabled: true, requireLocalEmailVerified: false },
    },
    rateLimit: {
      enabled: true,
      storage: "database",
      customRules: { "/sign-in/email": { window: 60, max: 5 } },
    },
    advanced: { ipAddress: { ipAddressHeaders: ["x-forwarded-for"] } },
    onAPIError: { errorURL: `${config.baseURL}/error` },
    databaseHooks: {
      verification: {
        create: {
          before: async (verification) => {
            if (!isAuthorizationCode(verification.value)) return;
            const latest = (verification.createdAt ?? new Date()).getTime() + AUTHORIZATION_CODE_SECONDS * 1000;
            if (verification.expiresAt.getTime() <= latest) return;
            return { data: { ...verification, expiresAt: new Date(latest) } };
          },
        },
      },
      account: {
        create: {
          before: async (account, ctx) => {
            if (account.providerId === "credential" || !ctx) return;
            const internal = ctx.context.internalAdapter;
            const user = await internal.findUserById(account.userId);
            if (!user || user.emailVerified) return;
            for (const unproven of await internal.findAccounts(user.id)) await internal.deleteAccount(unproven.id);
            await internal.deleteUserSessions(user.id);
          },
        },
      },
    },
    plugins: [
      jwt(),
      oauthProvider({
        loginPage: "/sign-in",
        consentPage: "/error",
        scopes: SCOPES,
        grantTypes: ["authorization_code", "refresh_token"],
        codeExpiresIn: SIGN_IN_PAGE_SECONDS,
        accessTokenExpiresIn: config.accessTokenSeconds,
        refreshTokenExpiresIn: 30 * 24 * 60 * 60,
        refreshTokenReuseInterval: config.refreshReuseSeconds,
        allowDynamicClientRegistration: false,
        clientPrivileges: () => false,
        cachedTrustedClients: new Set([CLIENT_ID]),
        validateRedirectUri: (uri, _registered, matchesRegistered) => matchesRegistered && isLoopbackCallback(uri),
      }),
      emailOTP({
        otpLength: 6,
        expiresIn: 10 * 60,
        storeOTP: "hashed",
        overrideDefaultEmailVerification: true,
        sendVerificationOTP: async ({ email, otp, type }) => {
          if (type === "email-verification") await mail(codeMessage(email, otp, "verify"));
          if (type === "forget-password") await mail(codeMessage(email, otp, "reset"));
        },
      }),
      genericOAuth({
        config: [
          {
            providerId: "google",
            name: "Google",
            clientId: config.google.clientId,
            clientSecret: config.google.clientSecret,
            discoveryUrl: config.google.discoveryUrl,
            scopes: ["openid", "email", "profile"],
            pkce: true,
            prompt: "select_account",
            requireIdTokenVerification: true,
          },
        ],
      }),
      haveIBeenPwned({
        customPasswordCompromisedMessage: "This password has appeared in a data leak. Choose a different one.",
      }),
      sync(database, CLIENT_ID),
    ],
  });
}

export type Auth = ReturnType<typeof createAuth>;
