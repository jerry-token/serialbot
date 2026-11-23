// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Fills the download table from the newest GitHub release (pre-releases
// included, since every release is a pre-release for now). Without JavaScript or if
// GitHub cannot be reached, every link still points at the releases page.

const RELEASES = "https://api.github.com/repos/jerry-token/serialbot/releases?per_page=1";

/** Size as GitHub shows it: binary megabytes, one decimal. */
export function formatSize(bytes) {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * For each file-name suffix, the release asset ending with it:
 * { suffix: { name, url, size } }. Suffixes without an asset are left out.
 */
export function pickAssets(release, suffixes) {
  const out = {};
  for (const suffix of suffixes) {
    const asset = (release?.assets ?? []).find((a) => a.name.endsWith(suffix));
    if (asset) {
      out[suffix] = { name: asset.name, url: asset.browser_download_url, size: formatSize(asset.size) };
    }
  }
  return out;
}

/** Updates the table rows (`tr[data-asset]`) and the release line. */
export function fillTable(doc, release) {
  const rows = [...doc.querySelectorAll("tr[data-asset]")];
  const found = pickAssets(release, rows.map((r) => r.dataset.asset));
  for (const row of rows) {
    const asset = found[row.dataset.asset];
    if (!asset) continue;
    const link = row.querySelector("a");
    link.href = asset.url;
    link.textContent = asset.name;
    row.querySelector(".size").textContent = asset.size;
  }
  const line = doc.querySelector(".release");
  if (line && release?.tag_name) {
    line.textContent = line.dataset.releaseLabel.replace("{tag}", release.tag_name);
  }
}

if (typeof document !== "undefined") {
  fetch(RELEASES)
    .then((r) => (r.ok ? r.json() : []))
    .then((list) => fillTable(document, list[0]))
    .catch(() => {
      // The links to the releases page stay.
    });
}
