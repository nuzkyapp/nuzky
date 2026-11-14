export function signedOAuthQuery(search: string | URLSearchParams): string | null {
  const params = new URLSearchParams(search);
  const signed = new Set(params.getAll("ba_param"));
  if (!params.has("sig") || signed.size === 0) return null;
  const kept = new URLSearchParams();
  for (const [key, value] of params) if (key === "sig" || key === "ba_param" || signed.has(key)) kept.append(key, value);
  return kept.toString();
}

export function withQuery(path: string, query: string | null): string {
  return query ? `${path}?${query}` : path;
}

export type SearchParams = Promise<Record<string, string | string[] | undefined>>;

export async function readSearch(searchParams: SearchParams): Promise<URLSearchParams> {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(await searchParams)) for (const one of [value ?? []].flat()) search.append(key, one);
  return search;
}
