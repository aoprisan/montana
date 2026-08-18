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
if (!Array.isArray(embedded.events) || embedded.events.length === 0) {
  throw new Error("index.html fallback does not contain any events");
}
for (const [name, calendar] of [["events.json", events], ["embedded fallback", embedded]]) {
  for (const event of calendar.events) {
    if (!/^2026-\d{2}-\d{2}$/.test(event.d) || Number.isNaN(Date.parse(`${event.d}T12:00:00Z`))) {
      throw new Error(`${name} contains an invalid date for ${event.name ?? "unnamed event"}`);
    }
    const url = new URL(event.url);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      throw new Error(`${name} contains an unsafe URL for ${event.name ?? "unnamed event"}`);
    }
  }
}
if (JSON.stringify(events.events) !== JSON.stringify(embedded.events)) {
  throw new Error("events.json and the embedded offline fallback have drifted");
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
    this.dataset = {};
    this.hidden = false;
    this.disabled = false;
    this.tabIndex = 0;
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
  ["list", "months", "q", "count", "showCancelled", "onlyUpcoming", "allTags", "nextRace", "gen", "data", "calendarGrid", "calendarTitle", "calendarPrev", "calendarNext", "selectionContent", "savedTabCount"].map(
    (id) => [id, new FakeElement(id)],
  ),
);
elements.get("data").textContent = JSON.stringify({
  generated: "2026-08-17",
  events: [
    { d: "2026-08-17", name: "Stale fallback", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
  ],
});
const currentEvents = {
  generated: "2026-08-18",
  events: [
    { d: "2026-08-16", d2: "2026-08-17", name: "Past", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-16", d2: "2026-08-18", name: "Ongoing", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-18", name: "Today", url: "#", loc: "—", county: "—", dist: "—", tags: [] },
    { d: "2026-08-19", name: "Future", url: "#", loc: "—", county: "—", dist: "10 / 20 km", tags: [] },
  ],
};
elements.get("showCancelled").setAttribute("aria-pressed", "true");
elements.get("onlyUpcoming").setAttribute("aria-pressed", "true");
elements.get("allTags").setAttribute("aria-pressed", "true");

class FixedDate extends Date {
  constructor(...args) {
    super(...(args.length ? args : ["2026-08-18T12:00:00"]));
  }
}

const storage = new Map();
const context = {
  Date: FixedDate,
  fetch: async () => ({ ok: true, json: async () => currentEvents }),
  console,
  history: { replaceState() {} },
  location: { protocol: "https:", pathname: "/", search: "", hash: "" },
  localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
  },
  window: { scrollTo() {} },
  document: {
    body: { dataset: {} },
    getElementById: (id) => elements.get(id),
    querySelectorAll: () => [],
  },
  navigator: {},
};
await runInNewContext(appScript, context);
await context.__appReady;

if (elements.get("list").innerHTML.includes("Stale fallback")) {
  throw new Error("application ignored the current events.json response");
}

if (!elements.get("count").innerHTML.startsWith("3 curse")) {
  throw new Error(`upcoming-only default returned the wrong event count: ${elements.get("count").innerHTML}`);
}
if (elements.get("onlyUpcoming").getAttribute("aria-pressed") !== "true") {
  throw new Error("upcoming-only filter is not selected by default");
}
if (elements.get("list").innerHTML.includes("Past") || !elements.get("list").innerHTML.includes("Ongoing")) {
  throw new Error("upcoming-only default mishandled past or ongoing events");
}

listeners.get("onlyUpcoming:click")({ currentTarget: elements.get("onlyUpcoming") });
if (!elements.get("count").innerHTML.startsWith("4 curse")) {
  throw new Error("disabling upcoming-only returned the wrong event count");
}
if (elements.get("onlyUpcoming").getAttribute("aria-pressed") !== "false") {
  throw new Error("upcoming-only toggle did not expose its disabled state");
}
if (!elements.get("list").innerHTML.includes("Past")) {
  throw new Error("disabling upcoming-only did not restore past events");
}

if (!html.includes('role="tab"') || !html.includes('id="panel-calendar"') || !html.includes('id="panel-selected"')) {
  throw new Error("three-view navigation is missing");
}
if (elements.get("list").innerHTML.includes("Preț") || elements.get("list").innerHTML.includes("RON")) {
  throw new Error("disabled pricing is still visible in the race list");
}

context.__calendarTrail.toggleSaved(currentEvents.events[3]);
if (!elements.get("selectionContent").innerHTML.includes("Curse selectate") || !elements.get("selectionContent").innerHTML.includes("10 / 20 km")) {
  throw new Error("saving a race did not update the simplified selection");
}
if (elements.get("selectionContent").innerHTML.includes("Preț") || elements.get("selectionContent").innerHTML.includes("RON")) {
  throw new Error("disabled pricing is still visible in the selection");
}

console.log(`Static PWA is valid (${events.events.length} events; tabs, saved races, and filters passed).`);
