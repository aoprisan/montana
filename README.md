# Munte 2026 — cursele montane din România

[Deschide aplicația](https://aoprisan.github.io/montana/)

Static PWA: calendarul curselor de alergare montană / trail / sky / vertical din România în 2026,
cu link către pagina oficială a fiecărei curse. ~116 evenimente, ianuarie–decembrie.

## Structură
- `app/` — tot ce trebuie publicat (GitHub Pages ready)
  - `index.html` — aplicația completă, cu datele **embedded** (merge și deschis direct din fișier)
  - `manifest.webmanifest`, `sw.js`, `icon.svg`, `icon-192/512.png` — PWA (installabil, offline)
  - `events.json` — aceleași date, ca fișier separat (sursa pentru viitorul scraper)
- `scraper/scrape.mjs` — parcat deocamdată: regenerator de events.json din calendarul
  comunitar vladcarbune.ro (Node ≥18 + cheerio). Nefolosit de aplicația statică.

## Dezvoltare locală

Aplicația nu are un pas de build și nu instalează dependențe. Pentru un preview local cu service worker:

```sh
python3 -m http.server 8000 --directory app
```

Deschide apoi `http://localhost:8000`. Verificarea folosită și în CI se rulează cu:

```sh
node scripts/validate.mjs
```

## Deploy

Workflow-ul [`.github/workflows/pages.yml`](.github/workflows/pages.yml) validează PWA-ul, împachetează
directorul `app/` și îl publică în GitHub Pages la fiecare push pe `main`. Poate fi pornit și manual
din fila **Actions**.

Pentru prima publicare, sursa din **Settings → Pages → Build and deployment** trebuie să fie
**GitHub Actions**. După deploy, site-ul este disponibil la
`https://aoprisan.github.io/montana/`.

## Date
Compilate manual (aug 2026) din: vladcarbune.ro/calendar-evenimente-alergare-2026,
fra.ro/competitii/calendar, runmap.ro. Cursele anulate (Făgăraș Rocks!, Up to Postăvaru,
Bate Toaca, Scaunul Domnului) sunt păstrate cu ștampila ANULAT.
Pentru actualizări editează `<script type="application/json" id="data">` din index.html.
