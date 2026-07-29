export type ServerResolution = { api: string | null; error: string | null };

export function resolveApiUrl(
  configured: string | undefined,
  options: { development: boolean; production: boolean }
): ServerResolution {
  const candidate = configured?.trim()
    || (options.development ? "http://localhost:8080" : "");
  if (!candidate) return {
    api: null,
    error: "Online play is not configured in this build. Install a release built with the public multiplayer server URL."
  };
  try {
    const url = new URL(candidate);
    const local = ["localhost", "127.0.0.1", "::1", "[::1]"].includes(url.hostname);
    if (!["http:", "https:"].includes(url.protocol)
      || (options.production && !local && url.protocol !== "https:"))
      throw new Error("Invalid multiplayer transport");
    return { api: url.toString().replace(/\/$/, ""), error: null };
  } catch {
    return {
      api: null,
      error: "This build contains an invalid multiplayer server URL. Please install a correctly configured release."
    };
  }
}

export function websocketUrl(api: string) {
  const url = new URL(api);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/api/online";
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function networkErrorMessage(error: unknown) {
  if (error instanceof TypeError && /fetch|network|load/i.test(error.message))
    return "Could not reach the multiplayer server. Check your internet connection or install the latest configured release.";
  return error instanceof Error ? error.message : String(error);
}
