export const FRESH_SIGN_IN_SECONDS = 10 * 60;

export function isFresh(createdAt: Date | string): boolean {
  return Date.now() - new Date(createdAt).getTime() < FRESH_SIGN_IN_SECONDS * 1000;
}
