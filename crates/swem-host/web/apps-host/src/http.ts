// The page's own calls to its host: JSON in, JSON out, and a refusal as the
// sentence the host said.

/// What the host said, without the kind of refusal it began with: the kind
/// is for whoever reads the status, the sentence is for the person.
const said = (words: string): string => words.replace(/^(invalid request|conflict|not found): /, "").replace(/^./, (first) => first.toUpperCase());

async function failure(response: Response): Promise<Error> {
  const body = await response.text();
  try {
    const parsed = JSON.parse(body) as { error?: string };
    return new Error(said(parsed.error ?? body));
  } catch {
    return new Error(body || `${response.status}`);
  }
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  if (!response.ok) throw await failure(response);
  return (await response.json()) as T;
}
