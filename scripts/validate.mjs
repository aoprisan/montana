#!/usr/bin/env node

import { access, readFile } from "node:fs/promises";
import { constants } from "node:fs";
import { resolve } from "node:path";

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

console.log(`Static PWA is valid (${events.events.length} events).`);
