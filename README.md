# Calendar Trail

[Deschide aplicația](https://aoprisan.github.io/montana/)

Static PWA: calendarul curselor de alergare montană / trail / sky / vertical din România în 2026
și 2027, cu link către pagina oficială a fiecărei curse. Peste 100 de evenimente, ianuarie–decembrie.

Aplicația are trei vizualizări: lista cronologică, calendarul lunar și selecția personală. Cursele
salvate sunt păstrate local în browser.

Sezonul se alege din selectorul de an, afișat doar când calendarul conține mai mult de un an.
Lista și calendarul lunar urmează sezonul selectat — săgețile calendarului trec dintr-un an în
altul — iar cursele salvate rămân vizibile împreună, indiferent de sezon. La deschidere este
selectat primul sezon cu curse viitoare.

## Structură
- `app/` — tot ce trebuie publicat (GitHub Pages ready)
  - `index.html` — aplicația și o copie embedded folosită doar ca fallback offline
  - `manifest.webmanifest`, `sw.js`, `icon.svg`, `icon-192/512.png` — PWA (installabil, offline)
  - `events.json` — sursa canonică încărcată de aplicație la fiecare pornire
- `scraper/` — worker Rust care actualizează sigur `events.json` din calendarul comunitar
- `deploy/montana-scraper.{service,timer}` — job systemd pornit zilnic pe VPS
- `docs/` — note tehnice; `typesafe-jev.md` evaluează clasificarea curselor cu un model de decizie

## Dezvoltare locală

Aplicația nu are un pas de build și nu instalează dependențe. Pentru un preview local cu service worker:

```sh
python3 -m http.server 8000 --directory app
```

Deschide apoi `http://localhost:8000`. Verificarea folosită și în CI se rulează cu:

```sh
node scripts/validate.mjs
```

Testele scraperului și o verificare live fără modificarea datelor:

```sh
cargo test --locked --manifest-path scraper/Cargo.toml
cargo run --locked --manifest-path scraper/Cargo.toml -- --dry-run
```

Implicit sunt actualizate sezoanele 2026 și 2027, fiecare din pagina comunitară a anului
respectiv. Un singur sezon se actualizează cu `--year 2027`; `--source` și `--fixture` cer un
singur `--year`. Sezoanele acceptate sunt listate în `SUPPORTED_YEARS` (`scraper/src/lib.rs`)
și în `supportedYears` (`scripts/validate.mjs`) — ambele trebuie extinse pentru un an nou.

## Deploy

Workflow-ul [`.github/workflows/pages.yml`](.github/workflows/pages.yml) validează PWA-ul, împachetează
directorul `app/` și îl publică în GitHub Pages la fiecare push pe `main`. Poate fi pornit și manual
din fila **Actions**.

Pentru prima publicare, sursa din **Settings → Pages → Build and deployment** trebuie să fie
**GitHub Actions**. După deploy, site-ul este disponibil la
`https://aoprisan.github.io/montana/`.

Pentru publicarea pe VPS-ul comun, cu Caddy, TLS automat și actualizare zilnică:

```sh
./scripts/deploy-vps.sh
```

Valorile implicite sunt `root@93.115.53.191` și `calendartrail.ro`; ambele pot fi
suprascrise prin argumente sau prin `MONTANA_DEPLOY_HOST` și `MONTANA_DEPLOY_DOMAIN`.
Mașina locală trebuie să aibă Docker, iar VPS-ul Caddy și systemd. Scriptul compilează un binar
Linux x86-64 într-un build Docker reproductibil, instalează serviciul cu utilizator neprivilegiat `montana`, îl rulează o dată și
activează timerul zilnic de la 05:17 (Europe/Bucharest), cu maximum 30 de minute de întârziere aleatorie.

Starea și ultima execuție pot fi verificate cu:

```sh
systemctl status montana-scraper.timer
journalctl -u montana-scraper.service -n 100 --no-pager
```

## Date
Datele inițiale au fost compilate manual (aug 2026) din vladcarbune.ro, fra.ro și runmap.ro.
Workerul actualizează intrările recunoscute și adaugă curse noi, dar păstrează intrările curate
manual care nu mai apar în sursa principală. Înainte de scriere verifică schema, URL-urile,
numărul de rezultate și abaterea față de calendarul existent.

Fiecare sezon este actualizat separat: cursele celorlalți ani sunt păstrate neatinse, iar
pragurile de siguranță se aplică doar sezonului scrapat. Un sezon pe care calendarul nu îl
conține încă (2027, până la publicarea paginii sursă) este sărit cu un avertisment, ca restul
sezoanelor să se actualizeze normal; dacă eșuează un sezon deja prezent în calendar, rularea
se oprește fără să scrie nimic. Scrierea este atomică, iar ultimele
12 versiuni sunt păstrate în `/opt/montana/backups`. La orice eroare rămâne publicat ultimul fișier valid.
