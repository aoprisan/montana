#!/usr/bin/env node
/**
 * Munte 2026 — scraper
 *
 * Fetches the community-maintained running calendar at
 * https://vladcarbune.ro/calendar-evenimente-alergare-2026/ , extracts events,
 * keeps only mountain/trail races, and writes ../app/events.json.
 *
 * Usage:
 *   node scrape.mjs                      # fetch live page, write ../app/events.json
 *   node scrape.mjs --fixture page.html  # parse a saved HTML file instead (testing)
 *   node scrape.mjs --dry                # print JSON to stdout, don't write
 *
 * Notes:
 * - Cancelled events (struck through / "Anulat") are kept with status:"anulat".
 * - (RM) entries (Republic of Moldova) are skipped.
 * - The mountain filter is keyword-based; tune INCLUDE / EXCLUDE below.
 */
import * as cheerio from "cheerio";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const SOURCE = "https://vladcarbune.ro/calendar-evenimente-alergare-2026/";
const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "app", "events.json");

const MONTHS = {
  ianuarie: 1, februarie: 2, martie: 3, aprilie: 4, mai: 5, iunie: 6,
  iulie: 7, august: 8, septembrie: 9, octombrie: 10, noiembrie: 11, decembrie: 12,
};

// --- mountain/trail filter -------------------------------------------------
const MOUNTAIN_WORDS = /trail|montan|munte|mountain|sky|vertical|uphill|everesting|ultra|forest|pădure|padure|maraton\s+piatra|rocks|creast|cheile|stairs/i;
// events that don't match keywords but ARE mountain races (match by name substring, lowercase)
const INCLUDE = [
  "să-run-i", "atinge omu", "omu marathon", "fuga lotrilor", "burduf challenge",
  "budureasa", "ecorun", "run to the hills", "turnu roșu", "obciniada",
  "transylvania 100", "ultrabug", "geopark mehedinți", "măcin", "sepsirun",
  "legendele nemirei", "cozia", "zărnești challenge", "szent gellért",
  "cindrel", "țara de piatră", "retezat", "oslea", "zarandia", "cugirace",
  "castelul din carpați", "șureanu", "heritage run", "hercules", "istrița",
  "mogoșa", "roșia montană", "igniș", "sinaia", "bate toaca", "brașov marathon",
  "european masters off road", "getic cross", "porolissum", "maratonul sării",
  "tarcău", "scaunul domnului", "bloodfluencer", "veverița", "dălhăuți",
  "cheile turzii", "maraton apuseni", "urme pe play", "bușteni",
];
// keyword matches that are actually road/city/charity events — drop them
const EXCLUDE = [
  "ultrarace 24h galați", "aleargă românia", "cursa imposibilă", "s24h",
  "ultra race romania", "mureș 24h", "campionatele nationale de ultraalergare",
  "the color run", "get muddy",
];
// ---------------------------------------------------------------------------

const TAG_RULES = [
  [/vertical|stairs|everesting|uphill/i, "vertical"],
  [/sky\s?race|sky marathon/i, "sky"],
  [/winter|iarnă|iarna/i, "iarnă"],
  [/night|noapte|by night/i, "noapte"],
  [/campionat|championship/i, "campionat"],
  [/utmb/i, "utmb"],
  [/obstacole/i, "obstacole"],
];

function iso(month, day) {
  return `2026-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function parseDate(text) {
  // "Iulie 25", "Mai 29-31", "Iulie 29-August 02", "Mai 29 - Iunie 01"
  const m = text.match(
    /(ianuarie|februarie|martie|aprilie|mai|iunie|iulie|august|septembrie|octombrie|noiembrie|decembrie)\s+(\d{1,2})(?:\s*[-–]\s*(?:(ianuarie|februarie|martie|aprilie|mai|iunie|iulie|august|septembrie|octombrie|noiembrie|decembrie)\s+)?(\d{1,2}))?/i
  );
  if (!m) return null;
  const m1 = MONTHS[m[1].toLowerCase()];
  const d1 = +m[2];
  let out = { d: iso(m1, d1) };
  if (m[4]) {
    const m2 = m[3] ? MONTHS[m[3].toLowerCase()] : m1;
    out.d2 = iso(m2, +m[4]);
  }
  return out;
}

function inferTags(name, dist) {
  const hay = `${name} ${dist}`;
  const tags = TAG_RULES.filter(([re]) => re.test(hay)).map(([, t]) => t);
  // ultra: any listed distance >= 42 km, or looped/timed formats
  const kms = [...dist.matchAll(/(\d+(?:[.,]\d+)?)\s*km/gi)].map((x) => parseFloat(x[1].replace(",", ".")));
  if (kms.some((k) => k >= 42) || /backyard|24h|nelimitat|etape/i.test(hay)) tags.push("ultra");
  return [...new Set(tags)];
}

function parseEvents(html) {
  const $ = cheerio.load(html);
  const events = [];
  $("p").each((_, p) => {
    const $p = $(p);
    const raw = $p.text().replace(/\s+/g, " ").trim();
    if (!raw.startsWith("•") && !/^\s*•/.test(raw)) return;
    if (/^\(?•?\s*\(RM\)/.test(raw) || raw.includes("(RM)")) return; // Moldova
    const date = parseDate(raw);
    if (!date) return;

    const $a = $p.find("strong a").first().length ? $p.find("strong a").first() : $p.find("a").first();
    if (!$a.length) return;
    const name = $a.text().replace(/\s+/g, " ").trim();
    const url = $a.attr("href");
    if (!name || !url) return;

    const cancelled = $p.find("del,s,strike").length > 0 || /anulat/i.test(raw);

    // tail after the name: ", Location XX – distances"
    const after = raw.split(name).slice(1).join(name).replace(/^\s*[,:]?\s*/, "");
    let [locPart, ...distParts] = after.split(/\s[–—-]\s/);
    let dist = distParts.join(" – ").replace(/anulat!?/gi, "").trim() || "—";
    locPart = (locPart || "").replace(/anulat!?/gi, "").trim();
    let county = "—";
    const cm = locPart.match(/\b([A-Z]{1,2})\b\s*$/);
    if (cm && cm[1] !== "I") { county = cm[1]; locPart = locPart.slice(0, cm.index).trim().replace(/,$/, ""); }
    const loc = locPart || "—";

    const hay = `${name} ${loc} ${dist}`.toLowerCase();
    const excluded = EXCLUDE.some((e) => hay.includes(e));
    const included = INCLUDE.some((i) => hay.includes(i));
    if (excluded) return;
    if (!included && !MOUNTAIN_WORDS.test(hay)) return;

    const ev = { ...date, name, url, loc, county, dist, tags: inferTags(name, dist) };
    if (cancelled) ev.status = "anulat";
    events.push(ev);
  });
  events.sort((a, b) => a.d.localeCompare(b.d) || a.name.localeCompare(b.name));
  return events;
}

async function main() {
  const args = process.argv.slice(2);
  const dry = args.includes("--dry");
  const fixIdx = args.indexOf("--fixture");
  let html;
  if (fixIdx !== -1) {
    html = readFileSync(args[fixIdx + 1], "utf8");
  } else {
    const res = await fetch(SOURCE, { headers: { "user-agent": "munte2026-scraper (personal project)" } });
    if (!res.ok) throw new Error(`fetch failed: ${res.status}`);
    html = await res.text();
  }
  const events = parseEvents(html);
  const out = {
    generated: new Date().toISOString().slice(0, 10),
    sources: [SOURCE, "https://www.fra.ro/competitii/calendar"],
    events,
  };
  const json = JSON.stringify(out, null, 1);
  if (dry) { console.log(json); }
  else { writeFileSync(OUT, json); console.log(`wrote ${OUT} (${events.length} events)`); }
  console.error(`parsed ${events.length} mountain/trail events`);
}
main().catch((e) => { console.error(e); process.exit(1); });
