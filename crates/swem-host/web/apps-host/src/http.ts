// The page's own calls to its host: JSON in, JSON out, and a refusal as the
// sentence the host said.

async function failure(response: Response): Promise<Error> {
  const body = await response.text();
  try {
    const parsed = JSON.parse(body) as { error?: string };
    return new Error(parsed.error ?? body);
  } catch {
    return new Error(body || `${response.status}`);
  }
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  if (!response.ok) throw await failure(response);
  return (await response.json()) as T;
}
