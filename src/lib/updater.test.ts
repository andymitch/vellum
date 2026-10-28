// The update check: which release a tester is offered (#291), and how often the
// app re-checks while it runs without nagging about a declined version (#292).
/// <reference types="bun-types" />
import { describe, expect, mock, setSystemTime, test } from "bun:test";

let asks: string[] = [];
let answer = false;
let available: string | null = null;
let checkedUrls: string[] = [];
mock.module("@tauri-apps/plugin-updater", () => ({ check: async () => null }));
mock.module("@tauri-apps/plugin-process", () => ({ relaunch: async () => {} }));
mock.module("@tauri-apps/plugin-dialog", () => ({
  ask: async (text: string) => (asks.push(text), answer),
  message: async () => {},
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args: { url: string }) => {
    if (cmd === "check_update_at") checkedUrls.push(args.url);
    return cmd === "check_update_at" ? available : undefined;
  },
}));
mock.module("./platform", () => ({
  appVersion: async () => "11.0.0-beta.4",
  isMacApp: true,
  isAndroidApp: false,
}));
mock.module("./beta-channel.svelte", () => ({ betaChannel: { enabled: true } }));

const release = (tag: string, prerelease: boolean, manifest = true) => ({
  tag_name: tag,
  prerelease,
  draft: false,
  body: "",
  html_url: "",
  assets: manifest ? [{ name: "latest.json", browser_download_url: `https://x/${tag}/latest.json` }] : [],
});
let releases = [release("v11-beta.4", true), release("v11", false), release("v10", false)];
globalThis.fetch = (async () => new Response(JSON.stringify(releases))) as unknown as typeof fetch;

const { cmpVersion, checkForBetaUpdate, checkForUpdatesIfDue } = await import("./updater");

test("cmpVersion counts missing components as zero", () => {
  expect(cmpVersion("3", "3.0.0")).toBe(0);
  expect(cmpVersion("3.1", "3.0.1")).toBeGreaterThan(0);
  expect(cmpVersion("10", "9.9")).toBeGreaterThan(0);
});

describe("beta channel", () => {
  const offered = async () => {
    checkedUrls = [];
    await checkForBetaUpdate(true);
    return checkedUrls[0];
  };

  test("offers the stable release that supersedes the beta", async () => {
    releases = [release("v11-beta.4", true), release("v11", false), release("v10", false)];
    expect(await offered()).toBe("https://x/v11/latest.json");
  });

  test("orders betas numerically and a hotfix below the next beta", async () => {
    releases = [release("v11.1", false), release("v12-beta.10", true), release("v12-beta.9", true)];
    expect(await offered()).toBe("https://x/v12-beta.10/latest.json");
  });

  test("skips a release with no updater manifest", async () => {
    releases = [release("v12-beta.1", true, false), release("v11", false)];
    expect(await offered()).toBe("https://x/v11/latest.json");
  });
});

describe("watching for updates", () => {
  test("re-checks at most every 30 minutes and asks once per declined version", async () => {
    releases = [release("v11", false)];
    available = "11.0.0";
    asks = [];
    let now = Date.now() + 24 * 3600 * 1000;
    setSystemTime(new Date(now));
    checkedUrls = [];
    await checkForUpdatesIfDue();
    expect(asks).toHaveLength(1);
    setSystemTime(new Date((now += 29 * 60 * 1000)));
    await checkForUpdatesIfDue();
    expect(checkedUrls).toHaveLength(1);
    setSystemTime(new Date((now += 2 * 60 * 1000)));
    await checkForUpdatesIfDue();
    expect(checkedUrls).toHaveLength(2);
    expect(asks).toHaveLength(1);
    await checkForBetaUpdate(true);
    expect(asks).toHaveLength(2);
    available = "11.1.0";
    await checkForUpdatesIfDue(true);
    expect(asks).toHaveLength(3);
    setSystemTime();
  });

  test("simultaneous triggers run one check", async () => {
    available = "12.0.0";
    asks = [];
    await Promise.all([checkForUpdatesIfDue(true), checkForUpdatesIfDue(true), checkForUpdatesIfDue(true)]);
    expect(asks).toHaveLength(1);
  });
});
