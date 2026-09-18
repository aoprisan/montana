# TypeSafe Jev — assessment for the calendar scraper

An assessment, not a shipped feature. Nothing here is implemented. Of the three
projects this was assessed against, this is the one where a typed decision model
clearly earns its place.

## What Jev is

[Jev](https://typesafe.ai) is TypeSafe AI's first *System One* model, in early
access since 2026-09-15. It generates **no text**. State goes in; typed answers
come out, every declared question evaluated in one round trip:

| Question | Answer |
|---|---|
| `noul` | probability of "yes" (0–1) |
| `choice` | selected label + per-label probabilities + confidence |
| `score` | probability-weighted level on an ordered rubric + confidence |

```
POST https://api.typesafe.ai/v1/systemone
Authorization: Bearer $TYPESAFE_API_KEY
{"state": …, "model": "jev-latest", "questions": {"id": {"type": …, "instructions": …, "criteria": …}}}
```

Vendor-reported: 70–500 ms per call, **$0.042 per million input tokens, output
free**.

## The rule

> **Jev routes text. Deterministic code decides numbers.** Never the reverse.

Here that means: the model decides *whether an event belongs in the calendar and
how it is tagged*. It never touches dates, URLs, counties, distances, the merge,
or the safety gates — those stay exactly as deterministic as they are today.

## The problem it solves

`scraper/src/lib.rs` decides "is this a mountain race?" three ways:

1. `EXCLUDE` — a denylist of road/ultra-road events that keep slipping through;
2. `INCLUDE` — **a hand-maintained allowlist of about fifty individual race
   names**;
3. `mountain_words` — a regex over `trail|montan|munte|mountain|sky|vertical|…`.

The allowlist exists precisely because the regex cannot judge. "Bate Toaca",
"Fuga Lotrilor", "Veverița" and "Cheile Turzii" say nothing about mountains in
their names, so each had to be typed in by hand, one at a time. Every new race
whose organiser picks a poetic name is silently dropped until somebody notices
and edits Rust. That is the treadmill, and it is exactly the judgement a `noul`
question makes without a list.

`infer_tags` has the same shape one level down: seven regexes for
vertical/sky/iarnă/noapte/campionat/utmb/obstacole, plus a distance rule for
`ultra`.

## The design

Per candidate event, one call, three questions:

```rust
use typesafe::{Choice, Noul, Questions, Score};

Questions::new()
    .with("is_mountain", Noul::new(
        "A mountain, trail, sky or vertical running race in Romania — run on \
         mountain or forest terrain, not a road or urban race")
        .when_true("Trail, mountain, sky, vertical, ultra on natural terrain")
        .when_false("Road race, city marathon, track, obstacle-course or charity fun run"))
    .with("discipline", Choice::new("Which discipline the event is")
        .option("trail",    "Trail or mountain running over natural terrain")
        .option("sky",      "Skyrunning: high altitude, steep technical ground")
        .option("vertical", "Vertical kilometre, uphill-only, stairs, everesting")
        .option("ultra",    "Distance beyond a marathon, backyard or multi-stage")
        .option("other",    "None of these"))
    .with("confidence_terrain", Score::new(
        "How clearly the text establishes mountain or forest terrain",
        ["unclear", "implied", "explicit"]));
```

`state` is the text the scraper already builds — name, location, distance — plus
the event URL's host, which is often the strongest hint:

```rust
client.system_one(
    json!({"name": name, "location": location, "distance": distance, "host": host}),
    questions,
)
```

Where it plugs in: `parse_events`, at the point where `explicitly_included ||
mountain_words.is_match(&haystack)` decides today. `EXCLUDE` stays in front of
it as a hard veto — it encodes decisions a human already made and should not be
re-litigated by a model. `INCLUDE` becomes a *cache seed* rather than a rule
(see below), and can shrink to nothing over time.

Admission rule: `is_mountain ≥ 0.8` admits, `≤ 0.2` rejects, and the band in
between is admitted **only** if `mountain_words` also matches — so the
regex becomes the tie-breaker it should always have been, and the model never
single-handedly adds an event it is unsure about.

Tags come from `discipline` plus the existing distance rule; the deterministic
`iarnă`/`noapte`/`campionat`/`utmb` regexes are precise already and stay.

## Non-negotiables

- **Cache every decision, and commit the cache.** Key on `url` + normalized
  `name`; store verdict, probability, model and the date it was asked, in a
  JSON file beside `events.json`. A re-run must produce the same calendar, so a
  `git diff` of `events.json` stays reviewable — the point of the whole scraper
  is that its output is a small, auditable file. A cached event is never asked
  about again, which also means a normal daily run makes **zero** calls: only
  genuinely new races cost anything.
- **Fall back, never fail.** No `TYPESAFE_API_KEY`, an API error, a timeout —
  the current `INCLUDE`/`mountain_words` path runs and the scrape completes.
  The daily timer must never go red because a third party is down.
- **Keep the safety gates.** `--minimum-events`, `--minimum-match-percent` and
  `--maximum-growth-percent` are the net under a bad classifier day as much as
  a bad scrape day. If the model ever went haywire, `maximum_growth_percent`
  catches it before `events.json` is written.
- **`--dry-run` stays honest.** It must show which events were admitted by the
  model and with what probability, so a human can inspect a season before it
  ships.
- **The app stays keyless.** `app/` is a static PWA on GitHub Pages with no
  build step and no dependencies. Nothing about this touches it; the model runs
  only in the scraper, on the VPS.

## Cost and wiring

`app/events.json` holds 116 events. A full cold classification is on the order
of a cent; with the cache, routine runs cost nothing at all, because only new
races are asked about.

The systemd unit has no `EnvironmentFile` today, so one line is needed:

```ini
EnvironmentFile=-/etc/montana/scraper.env   # TYPESAFE_API_KEY=…
```

The `-` prefix keeps the unit starting when the file is absent, which is the
same "degrade, don't fail" posture as the code path.

## Compatibility

Rust client: [`typesafe-ai-sdk`](https://crates.io/crates/typesafe-ai-sdk)
`0.1.0` (`aoprisan/typesafe-ai-rust-sdk`), edition 2024, MSRV 1.88, `blocking`
feature — which matches how the scraper already uses reqwest.

```toml
typesafe-ai-sdk = { version = "0.1", features = ["blocking"] }
```

Verified: `cargo check` passes on this crate with that dependency added. The SDK
is on reqwest 0.13 and the scraper on 0.12; both live in the graph without
conflict, at 206 → 247 transitive crates. The scraper is a server-side binary
run once a day, so build size is not a constraint here.

Tests stay offline: the SDK honours `TYPESAFE_BASE_URL`, so a fixture server
covers the classification path and `scripts/validate.mjs` and CI never need a
key or a network.

## Out of bounds

- Calling the API from `app/` — the key would be public, and the app has no
  build step by design.
- Letting the model write dates, URLs, counties or distances.
- Letting the model override `EXCLUDE`.
- Skipping the cache. An uncached model call makes `events.json` non-reproducible,
  which would undo the main safety property of this repo.

## Status

Assessed 2026-09-18. Nothing built. This is the one of the three assessed
projects with a green light: the treadmill is real, the blast radius is small,
the fallback is the current behaviour, and the safety gates already exist.
