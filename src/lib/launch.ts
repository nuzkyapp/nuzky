/**
 * Read before any other module runs (src/main.tsx imports it first), so nothing this launch writes counts: whether
 * this profile kept anything of Nuzky's from an earlier launch.
 */
export const usedBefore = Object.keys(localStorage).some((key) => key.startsWith("nuzky."));
