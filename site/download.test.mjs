// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// node --test site/
import assert from "node:assert/strict";
import { test } from "node:test";
import { fillTable, formatSize, pickAssets } from "./download.js";

const release = {
  tag_name: "v0.1.0-alpha.5",
  assets: [
    { name: "SerialBot_0.1.0_x64-setup.exe", size: 4718592, browser_download_url: "https://x/setup" },
    { name: "SerialBot_0.1.0_arm64.deb", size: 5872025, browser_download_url: "https://x/arm" },
    { name: "SerialBot_0.1.0_amd64.deb", size: 6186598, browser_download_url: "https://x/amd" },
  ],
};

test("sizes read like the release page", () => {
  assert.equal(formatSize(4718592), "4.5 MB");
  assert.equal(formatSize(0), "0.0 MB");
});

test("assets are matched by suffix, missing ones left out", () => {
  const found = pickAssets(release, ["_amd64.deb", "_arm64.deb", "_universal.dmg"]);
  assert.deepEqual(Object.keys(found).sort(), ["_amd64.deb", "_arm64.deb"]);
  assert.equal(found["_amd64.deb"].url, "https://x/amd");
  assert.deepEqual(pickAssets(undefined, ["_amd64.deb"]), {});
});

test("the app's and the CLI's packages are told apart", () => {
  const both = {
    assets: [
      { name: "serialbot-cli-0.3.0-linux-amd64.deb", size: 1, browser_download_url: "https://x/cli" },
      { name: "SerialBot_0.3.0_amd64.deb", size: 2, browser_download_url: "https://x/app" },
    ],
  };
  const found = pickAssets(both, ["_amd64.deb", "-linux-amd64.deb"]);
  assert.equal(found["_amd64.deb"].url, "https://x/app");
  assert.equal(found["-linux-amd64.deb"].url, "https://x/cli");
});

test("the table gets links, names, sizes and the tag", () => {
  const row = (suffix) => {
    const link = { href: "https://github.com/x/releases", textContent: "placeholder" };
    const size = { textContent: "" };
    return {
      dataset: { asset: suffix },
      querySelector: (sel) => (sel === "a" ? link : size),
      link,
      size,
    };
  };
  const rows = [row("_x64-setup.exe"), row("_universal.dmg")];
  const line = { dataset: { releaseLabel: "Latest release: {tag}" }, textContent: "" };
  const doc = { querySelectorAll: () => rows, querySelector: () => line };
  fillTable(doc, release);
  assert.equal(rows[0].link.href, "https://x/setup");
  assert.equal(rows[0].link.textContent, "SerialBot_0.1.0_x64-setup.exe");
  assert.equal(rows[0].size.textContent, "4.5 MB");
  // No asset: the link to the releases page stays.
  assert.equal(rows[1].link.href, "https://github.com/x/releases");
  assert.equal(line.textContent, "Latest release: v0.1.0-alpha.5");
});
