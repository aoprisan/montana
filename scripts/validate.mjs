#!/usr/bin/env node

import { access, readFile } from "node:fs/promises";
import { constants } from "node:fs";
import { resolve } from "node:path";
import { runInNewContext } from "node:vm";

const root = resolve(import.meta.dirname, "..");
const app = resolve(root, "app");
const requiredFiles = [
  "index.html",
  "events.json",
  "manifest.webmanifest",
  "sw.js",
  "icon.svg",
  "icon-192.png",
  "icon-512.png",
];

await Promise.all(
  requiredFiles.map((file) => access(resolve(app, file), constants.R_OK)),
);

const [html, eventsText, manifestText, serviceWorker] = await Promise.all([
  readFile(resolve(app, "index.html"), "utf8"),
  readFile(resolve(app, "events.json"), "utf8"),
  readFile(resolve(app, "manifest.webmanifest"), "utf8"),
  readFile(resolve(app, "sw.js"), "utf8"),
]);

const events = JSON.parse(eventsText);
const manifest = JSON.parse(manifestText);
const embeddedMatch = html.match(
  /<script\s+type="application\/json"\s+id="data">([\s\S]*?)<\/script>/,
);

if (!embeddedMatch) {
  throw new Error("index.html is missing the embedded event data");
}

const embedded = JSON.parse(embeddedMatch[1]);
if (!Array.isArray(events.events) || events.events.length === 0) {
  throw new Error("events.json does not contain any events");
}
if (JSON.stringify(events) !== JSON.stringify(embedded)) {
  throw new Error("events.json and the event data embedded in index.html differ");
}
if (!html.includes('rel="manifest" href="manifest.webmanifest"')) {
  throw new Error("index.html does not link the web app manifest");
}
if (!html.includes('navigator.serviceWorker.register("sw.js")')) {
  throw new Error("index.html does not register the service worker");
}
if (manifest.start_url !== "./" || manifest.scope !== "./") {
  throw new Error("manifest start_url and scope must remain path-relative for Pages");
}

for (const icon of manifest.icons ?? []) {
  await access(resolve(app, icon.src), constants.R_OK);
}

for (const file of [
  "index.html",
  "events.json",
  "manifest.webmanifest",
  "icon.svg",
  "icon-192.png",
  "icon-512.png",
]) {
  if (!serviceWorker.includes(`"${file}"`)) {
    throw new Error(`service worker app shell is missing ${file}`);
  }
}

const appScript = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].at(-1)?.[1];
if (!appScript) {
  throw new Error("index.html is missing its application script");
}

const listeners = new Map();
class FakeElement {
  constructor(id) {
    this.id = id;
    this.attributes = new Map();
    this.innerHTML = "";
    this.textContent = "";
    this.value = "";
  }

  addEventListener(type, listener) {
    listeners.set(`${this.id}:${type}`, listener);
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  setAttribute(name, value) {
    this.attributes.set(name, value);
  }

  querySelectorAll() {
    return [];
  }
}

const elements = new Map(
  ["list", "ridge", "q", "count", "showCancelled", "onlyUpcoming", "gen", "data"].map(
    (id) => [id, new FakeElement(id)],
  ),
);
elements.get("data").textContent = JSON.stringify({
  generated: "2026-08-18",
  events: [
    { d: "2026-08-16", d2: "2026-08-17", name: "Past", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-16", d2: "2026-08-18", name: "Ongoing", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-18", name: "Today", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-19", name: "Future", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
  ],
});
elements.get("showCancelled").setAttribute("aria-pressed", "true");
elements.get("onlyUpcoming").setAttribute("aria-pressed", "false");

class FixedDate extends Date {
  constructor(...args) {
    super(...(args.length ? args : ["2026-08-18T12:00:00"]));
  }
}

runInNewContext(appScript, {
  Date: FixedDate,
  document: {
    getElementById: (id) => elements.get(id),
    querySelectorAll: () => [],
  },
  navigator: {},
});

listeners.get("onlyUpcoming:click")({ currentTarget: elements.get("onlyUpcoming") });
if (elements.get("count").textContent !== "3 / 4") {
  throw new Error("upcoming-only toggle returned the wrong event count");
}
if (elements.get("onlyUpcoming").getAttribute("aria-pressed") !== "true") {
  throw new Error("upcoming-only toggle did not expose its pressed state");
}
if (elements.get("list").innerHTML.includes("Past") || !elements.get("list").innerHTML.includes("Ongoing")) {
  throw new Error("upcoming-only toggle mishandled past or ongoing events");
}

console.log(`Static PWA is valid (${events.events.length} events; upcoming-only toggle passed).`);
