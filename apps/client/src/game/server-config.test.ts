import { describe, expect, it } from "vitest";
import { networkErrorMessage, resolveApiUrl, websocketUrl } from "./server-config";

describe("resolveApiUrl", () => {
  it("uses localhost during development", () => {
    expect(resolveApiUrl(undefined, { development: true, production: false }))
      .toEqual({ api: "http://localhost:8080", error: null });
  });

  it("reports a missing production endpoint", () => {
    const result = resolveApiUrl(undefined, { development: false, production: true });
    expect(result.api).toBeNull();
    expect(result.error).toContain("not configured");
  });

  it("normalizes a trailing slash", () => {
    expect(resolveApiUrl("https://api.example.com/", {
      development: false, production: true
    }).api).toBe("https://api.example.com");
  });

  it.each(["ftp://api.example.com", "ws://api.example.com", "not a url"])(
    "rejects invalid endpoint %s",
    endpoint => {
      const result = resolveApiUrl(endpoint, { development: false, production: true });
      expect(result.api).toBeNull();
      expect(result.error).toContain("invalid");
    }
  );

  it("rejects insecure public production endpoints", () => {
    expect(resolveApiUrl("http://api.example.com", {
      development: false, production: true
    }).api).toBeNull();
  });

  it.each(["http://localhost:8080", "http://127.0.0.1:8080", "http://[::1]:8080"])(
    "permits local endpoint %s for packaged testing",
    endpoint => {
      expect(resolveApiUrl(endpoint, {
        development: false, production: true
      }).error).toBeNull();
    }
  );
});

describe("websocketUrl", () => {
  it("converts HTTPS to WSS and removes unrelated URL state", () => {
    expect(websocketUrl("https://api.example.com/base?token=no#fragment"))
      .toBe("wss://api.example.com/api/online");
  });

  it("converts local HTTP to WS", () => {
    expect(websocketUrl("http://localhost:8080")).toBe("ws://localhost:8080/api/online");
  });
});

describe("networkErrorMessage", () => {
  it("turns fetch failures into an actionable message", () => {
    expect(networkErrorMessage(new TypeError("Failed to fetch"))).toContain(
      "Could not reach the multiplayer server"
    );
  });

  it("preserves application errors and unknown values", () => {
    expect(networkErrorMessage(new Error("Session expired"))).toBe("Session expired");
    expect(networkErrorMessage("offline")).toBe("offline");
  });
});
