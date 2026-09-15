use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// Seasons the calendar publishes, oldest first.
pub const SUPPORTED_YEARS: &[i32] = &[2026, 2027];
const SOURCE_TEMPLATE: &str = "https://vladcarbune.ro/calendar-evenimente-alergare-{year}/";

/// Community calendar page for a season.
pub fn default_source(year: i32) -> String {
    SOURCE_TEMPLATE.replace("{year}", &year.to_string())
}

/// Season an event belongs to, taken from its start date.
pub fn event_year(event: &Event) -> Option<i32> {
    NaiveDate::parse_from_str(&event.d, "%Y-%m-%d")
        .ok()
        .map(|date| date.year())
}
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

const INCLUDE: &[&str] = &[
    "să-run-i",
    "atinge omu",
    "omu marathon",
    "fuga lotrilor",
    "burduf challenge",
    "budureasa",
    "ecorun",
    "run to the hills",
    "turnu roșu",
    "obciniada",
    "transylvania 100",
    "ultrabug",
    "geopark mehedinți",
    "măcin",
    "sepsirun",
    "legendele nemirei",
    "cozia",
    "zărnești challenge",
    "szent gellért",
    "cindrel",
    "țara de piatră",
    "retezat",
    "oslea",
    "zarandia",
    "cugirace",
    "castelul din carpați",
    "șureanu",
    "heritage run",
    "hercules",
    "istrița",
    "mogoșa",
    "roșia montană",
    "igniș",
    "sinaia",
    "bate toaca",
    "brașov marathon",
    "european masters off road",
    "getic cross",
    "porolissum",
    "maratonul sării",
    "tarcău",
    "scaunul domnului",
    "bloodfluencer",
    "veverița",
    "dălhăuți",
    "cheile turzii",
    "maraton apuseni",
    "urme pe play",
    "bușteni",
];

const EXCLUDE: &[&str] = &[
    "ultrarace 24h galați",
    "aleargă românia",
    "cursa imposibilă",
    "s24h",
    "ultra race romania",
    "mureș 24h",
    "campionatele nationale de ultraalergare",
    "the color run",
    "get muddy",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Calendar {
    pub generated: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub events: Vec<Event>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Event {
    pub d: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d2: Option<String>,
    pub name: String,
    pub url: String,
    pub loc: String,
    pub county: String,
    pub dist: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct SafetyPolicy {
    pub minimum_scraped_events: usize,
    pub minimum_match_percent: u8,
    pub maximum_growth_percent: u16,
    pub force: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergeReport {
    pub scraped: usize,
    pub matched: usize,
    pub added: usize,
    pub retained: usize,
    pub changed: usize,
    pub carried: usize,
}

/// Outcome of a multi-season run: what was refreshed, and what the source did not serve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefreshSummary {
    pub refreshed: Vec<(i32, MergeReport)>,
    pub skipped: Vec<(i32, String)>,
}

/// Refresh each season in turn against a single calendar.
///
/// `load` returns the source URL and HTML for a season. A season the calendar already covers
/// must keep refreshing, so its failure aborts the run; a season the source has not published
/// yet is skipped, leaving the other seasons free to refresh.
pub fn refresh_seasons<F>(
    baseline: &Calendar,
    years: &[i32],
    policy: &SafetyPolicy,
    mut load: F,
) -> Result<(Calendar, RefreshSummary)>
where
    F: FnMut(i32) -> Result<(String, String)>,
{
    let mut seasons = years.to_vec();
    seasons.sort_unstable();
    seasons.dedup();
    ensure!(!seasons.is_empty(), "at least one season is required");

    let mut calendar = baseline.clone();
    let mut summary = RefreshSummary::default();

    for year in seasons {
        let refreshed = load(year).and_then(|(source, html)| {
            let scraped = parse_events(&html, year)?;
            merge_with_baseline(scraped, &calendar, year, &source, policy)
        });
        match refreshed {
            Ok((merged, report)) => {
                calendar = merged;
                summary.refreshed.push((year, report));
            }
            Err(error) => {
                let covered = calendar
                    .events
                    .iter()
                    .any(|event| event_year(event) == Some(year));
                if covered {
                    return Err(error.context(format!("season {year} could not be refreshed")));
                }
                summary.skipped.push((year, format!("{error:#}")));
            }
        }
    }
    ensure!(
        !summary.refreshed.is_empty(),
        "no season could be refreshed"
    );
    Ok((calendar, summary))
}

pub fn fetch_html(source: &str) -> Result<String> {
    let parsed = Url::parse(source).context("invalid source URL")?;
    ensure!(parsed.scheme() == "https", "source URL must use HTTPS");

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("could not construct HTTP client")?;
    let response = client
        .get(source)
        .header(
            reqwest::header::USER_AGENT,
            "calendartrail.ro scraper/0.1 (+https://calendartrail.ro)",
        )
        .send()
        .context("source request failed")?
        .error_for_status()
        .context("source returned an unsuccessful status")?;

    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        let content_type = content_type.to_str().unwrap_or_default();
        ensure!(
            content_type.starts_with("text/html"),
            "source returned unexpected content type {content_type}"
        );
    }

    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_RESPONSE_BYTES as u64,
            "source response is too large ({length} bytes)"
        );
    }
    let bytes = response.bytes().context("could not read source response")?;
    ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "source response exceeds {} bytes",
        MAX_RESPONSE_BYTES
    );
    String::from_utf8(bytes.to_vec()).context("source response is not UTF-8")
}

pub fn parse_events(html: &str, year: i32) -> Result<Vec<Event>> {
    let document = Html::parse_document(html);
    let paragraph = Selector::parse("p").expect("static paragraph selector");
    let preferred_anchor = Selector::parse("strong a").expect("static preferred anchor selector");
    let anchor = Selector::parse("a").expect("static anchor selector");
    let cancelled_selector = Selector::parse("del, s, strike").expect("static cancel selector");
    let mountain_words = Regex::new(
        r"(?i)trail|montan|munte|mountain|sky|vertical|uphill|everesting|ultra|forest|pădure|padure|maraton\s+piatra|rocks|creast|cheile|stairs",
    )?;
    let county_pattern = Regex::new(r"\b([A-Z]{1,2})\s*$")?;
    let mut events = Vec::new();

    for p in document.select(&paragraph) {
        let raw = normalize_space(&p.text().collect::<Vec<_>>().join(" "));
        if !raw.starts_with('•') || raw.contains("(RM)") {
            continue;
        }
        let Some((start, end)) = parse_date(&raw, year)? else {
            continue;
        };
        let event_anchor = p
            .select(&preferred_anchor)
            .next()
            .or_else(|| p.select(&anchor).next());
        let Some(event_anchor) = event_anchor else {
            continue;
        };
        let name = normalize_space(&event_anchor.text().collect::<Vec<_>>().join(" "));
        let Some(url) = event_anchor.value().attr("href").map(str::trim) else {
            continue;
        };
        if name.is_empty() || url.is_empty() {
            continue;
        }

        let cancelled =
            p.select(&cancelled_selector).next().is_some() || raw.to_lowercase().contains("anulat");
        let after_name = raw
            .find(&name)
            .map(|position| &raw[position + name.len()..])
            .unwrap_or_default()
            .trim_start_matches(|character: char| {
                character.is_whitespace() || matches!(character, ',' | ':')
            });
        let (mut location, distance) = split_location_distance(after_name);
        location = remove_cancelled_marker(&location);
        let distance = remove_cancelled_marker(&distance);
        let mut county = "—".to_owned();
        if let Some(captures) = county_pattern.captures(&location) {
            let code = captures.get(1).expect("county capture").as_str();
            if code != "I" {
                county = code.to_owned();
                let start = captures.get(0).expect("full county capture").start();
                location = location[..start]
                    .trim()
                    .trim_end_matches(',')
                    .trim()
                    .to_owned();
            }
        }
        if location.is_empty() {
            location = "—".to_owned();
        }

        let haystack = format!("{name} {location} {distance}").to_lowercase();
        if EXCLUDE.iter().any(|entry| haystack.contains(entry)) {
            continue;
        }
        let explicitly_included = INCLUDE.iter().any(|entry| haystack.contains(entry));
        if !explicitly_included && !mountain_words.is_match(&haystack) {
            continue;
        }

        let event = Event {
            d: start.format("%Y-%m-%d").to_string(),
            d2: end.map(|date| date.format("%Y-%m-%d").to_string()),
            name,
            url: url.to_owned(),
            loc: location,
            county,
            dist: if distance.is_empty() {
                "—".to_owned()
            } else {
                distance
            },
            tags: infer_tags(&haystack),
            status: cancelled.then(|| "anulat".to_owned()),
            note: None,
            extra: BTreeMap::new(),
        };
        validate_event(&event, year)?;
        events.push(event);
    }

    events.sort_by(|left, right| left.d.cmp(&right.d).then(left.name.cmp(&right.name)));
    reject_duplicates(&events)?;
    Ok(events)
}

/// Refresh a single season in `baseline`, leaving every other season untouched.
pub fn merge_with_baseline(
    scraped: Vec<Event>,
    baseline: &Calendar,
    year: i32,
    source: &str,
    policy: &SafetyPolicy,
) -> Result<(Calendar, MergeReport)> {
    for event in &scraped {
        validate_event(event, year)?;
    }

    let (season, carried): (Vec<&Event>, Vec<&Event>) = baseline
        .events
        .iter()
        .partition(|event| event_year(event) == Some(year));

    // A season the calendar does not cover yet has nothing to be measured against, and a
    // sparse early calendar must not be held to the count expected of a full one.
    let minimum_scraped = if season.is_empty() {
        0
    } else {
        policy.minimum_scraped_events.min(season.len())
    };
    ensure!(
        policy.force || scraped.len() >= minimum_scraped,
        "safety gate rejected {} scraped events for {year}; minimum is {minimum_scraped}",
        scraped.len()
    );

    let mut url_counts = HashMap::new();
    let mut baseline_by_name_date = HashMap::new();
    for (index, event) in season.iter().enumerate() {
        *url_counts.entry(url_key(&event.url)).or_insert(0_usize) += 1;
        baseline_by_name_date
            .entry(name_date_key(event))
            .or_insert(index);
    }
    let baseline_by_unique_url = season
        .iter()
        .enumerate()
        .filter_map(|(index, event)| {
            let key = url_key(&event.url);
            (url_counts.get(&key) == Some(&1)).then_some((key, index))
        })
        .collect::<HashMap<_, _>>();

    let mut matched_indexes = HashSet::new();
    let mut merged = Vec::with_capacity(baseline.events.len() + scraped.len());
    let mut report = MergeReport {
        scraped: scraped.len(),
        carried: carried.len(),
        ..MergeReport::default()
    };

    for mut incoming in scraped {
        let baseline_index = baseline_by_name_date
            .get(&name_date_key(&incoming))
            .or_else(|| baseline_by_unique_url.get(&url_key(&incoming.url)))
            .copied();
        if let Some(index) = baseline_index {
            let previous = season[index];
            matched_indexes.insert(index);
            report.matched += 1;
            preserve_curated_fields(&mut incoming, previous);
            if incoming != *previous {
                report.changed += 1;
            }
        } else {
            report.added += 1;
        }
        merged.push(incoming);
    }

    for (index, event) in season.iter().enumerate() {
        if !matched_indexes.contains(&index) {
            merged.push((*event).clone());
            report.retained += 1;
        }
    }
    for event in &carried {
        merged.push((*event).clone());
    }
    merged.sort_by(|left, right| left.d.cmp(&right.d).then(left.name.cmp(&right.name)));
    reject_duplicates(&merged)?;

    if !season.is_empty() && !policy.force {
        let match_percent = report.matched * 100 / season.len();
        ensure!(
            match_percent >= policy.minimum_match_percent as usize,
            "safety gate rejected {match_percent}% baseline matches for {year}; minimum is {}%",
            policy.minimum_match_percent
        );
        let growth_percent = report.added * 100 / season.len();
        ensure!(
            growth_percent <= policy.maximum_growth_percent as usize,
            "safety gate rejected {growth_percent}% new events for {year}; maximum is {}%",
            policy.maximum_growth_percent
        );
    }

    let mut sources = baseline.sources.clone();
    if !sources.iter().any(|existing| existing == source) {
        sources.push(source.to_owned());
    }
    let calendar = Calendar {
        generated: Utc::now().date_naive().format("%Y-%m-%d").to_string(),
        sources,
        events: merged,
        extra: baseline.extra.clone(),
    };
    validate_calendar(&calendar)?;
    Ok((calendar, report))
}

pub fn read_calendar(path: &Path) -> Result<Calendar> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("could not read baseline {}", path.display()))?;
    let calendar = serde_json::from_str(&contents)
        .with_context(|| format!("invalid calendar JSON in {}", path.display()))?;
    Ok(calendar)
}

pub fn write_calendar_atomic(
    path: &Path,
    calendar: &Calendar,
    backup_directory: Option<&Path>,
) -> Result<()> {
    validate_calendar(calendar)?;
    let serialized = serde_json::to_string_pretty(calendar)? + "\n";
    let parent = path
        .parent()
        .context("output path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;

    if path.exists()
        && let Some(backup_directory) = backup_directory
    {
        fs::create_dir_all(backup_directory).with_context(|| {
            format!(
                "could not create backup directory {}",
                backup_directory.display()
            )
        })?;
        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup_path = backup_directory.join(format!("events-{stamp}.json"));
        fs::copy(path, &backup_path)
            .with_context(|| format!("could not create backup {}", backup_path.display()))?;
        prune_backups(backup_directory, 12)?;
    }

    let temporary = temporary_path(path);
    let write_result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .with_context(|| format!("could not create {}", temporary.display()))?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("could not atomically replace {}", path.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

pub fn validate_calendar(calendar: &Calendar) -> Result<()> {
    ensure!(!calendar.events.is_empty(), "calendar must contain events");
    NaiveDate::parse_from_str(&calendar.generated, "%Y-%m-%d")
        .context("generated must be an ISO date")?;
    for event in &calendar.events {
        let year = NaiveDate::parse_from_str(&event.d, "%Y-%m-%d")
            .with_context(|| format!("invalid start date for {}", event.name))?
            .year();
        validate_event(event, year)?;
    }
    reject_duplicates(&calendar.events)
}

fn parse_date(text: &str, year: i32) -> Result<Option<(NaiveDate, Option<NaiveDate>)>> {
    let months = "ianuarie|februarie|martie|aprilie|mai|iunie|iulie|august|septembrie|octombrie|noiembrie|decembrie";
    let pattern = Regex::new(&format!(
        r"(?i)({months})\s+(\d{{1,2}})(?:\s*[-–]\s*(?:({months})\s+)?(\d{{1,2}}))?"
    ))?;
    let Some(captures) = pattern.captures(text) else {
        return Ok(None);
    };
    let start_month = month_number(&captures[1]).expect("regex only accepts known months");
    let start_day: u32 = captures[2].parse()?;
    let start = NaiveDate::from_ymd_opt(year, start_month, start_day)
        .with_context(|| format!("invalid start date {} {}", &captures[1], start_day))?;
    let end = if let Some(day) = captures.get(4) {
        let end_month = captures
            .get(3)
            .and_then(|value| month_number(value.as_str()))
            .unwrap_or(start_month);
        let end_day: u32 = day.as_str().parse()?;
        let end = NaiveDate::from_ymd_opt(year, end_month, end_day)
            .with_context(|| format!("invalid end date month {end_month} day {end_day}"))?;
        ensure!(end >= start, "date range ends before it starts: {text}");
        Some(end)
    } else {
        None
    };
    Ok(Some((start, end)))
}

fn month_number(month: &str) -> Option<u32> {
    match month.to_lowercase().as_str() {
        "ianuarie" => Some(1),
        "februarie" => Some(2),
        "martie" => Some(3),
        "aprilie" => Some(4),
        "mai" => Some(5),
        "iunie" => Some(6),
        "iulie" => Some(7),
        "august" => Some(8),
        "septembrie" => Some(9),
        "octombrie" => Some(10),
        "noiembrie" => Some(11),
        "decembrie" => Some(12),
        _ => None,
    }
}

fn split_location_distance(text: &str) -> (String, String) {
    let separator = Regex::new(r"\s*[–—]\s*|\s+-\s+").expect("static separator regex");
    if let Some(found) = separator.find(text) {
        let location = text[..found.start()].trim().to_owned();
        let distance = normalize_distance(&text[found.end()..]);
        return (location, distance);
    }
    (text.trim().to_owned(), "—".to_owned())
}

fn normalize_distance(distance: &str) -> String {
    let between_distances = Regex::new(r"(?i)\s*km\s*,\s*").expect("static distance regex");
    let before_unit = Regex::new(r"(?i)(\d)km\b").expect("static unit regex");
    let distance = between_distances.replace_all(distance.trim(), " / ");
    before_unit.replace_all(&distance, "$1 km").into_owned()
}

fn remove_cancelled_marker(value: &str) -> String {
    Regex::new(r"(?i)\s*anulat!?\s*")
        .expect("static cancellation regex")
        .replace_all(value, " ")
        .trim()
        .to_owned()
}

fn infer_tags(haystack: &str) -> Vec<String> {
    let rules = [
        (r"vertical|stairs|everesting|uphill", "vertical"),
        (r"sky\s?race|sky marathon", "sky"),
        (r"winter|iarnă|iarna", "iarnă"),
        (r"night|noapte|by night", "noapte"),
        (r"campionat|championship", "campionat"),
        (r"utmb", "utmb"),
        (r"obstacole", "obstacole"),
    ];
    let mut tags = Vec::new();
    for (pattern, tag) in rules {
        if Regex::new(pattern)
            .expect("static tag regex")
            .is_match(haystack)
        {
            tags.push(tag.to_owned());
        }
    }
    let distance = Regex::new(r"(?i)(\d+(?:[.,]\d+)?)(?:\s*km|\s*/)").expect("static km regex");
    let ultra_distance = distance.captures_iter(haystack).any(|captures| {
        captures[1]
            .replace(',', ".")
            .parse::<f32>()
            .is_ok_and(|km| km >= 42.0)
    });
    if ultra_distance
        || Regex::new(r"backyard|24h|nelimitat|etape")
            .expect("static ultra regex")
            .is_match(haystack)
    {
        tags.push("ultra".to_owned());
    }
    let mut seen = HashSet::new();
    tags.retain(|tag| seen.insert(tag.clone()));
    tags
}

fn preserve_curated_fields(incoming: &mut Event, previous: &Event) {
    if incoming.loc == "—" && previous.loc != "—" {
        incoming.loc.clone_from(&previous.loc);
    }
    if incoming.county == "—" && previous.county != "—" {
        incoming.county.clone_from(&previous.county);
    }
    if incoming.dist == "—" && previous.dist != "—" {
        incoming.dist.clone_from(&previous.dist);
    }
    incoming.note.clone_from(&previous.note);
    incoming.extra.clone_from(&previous.extra);
    if previous.status.as_deref() == Some("anulat") {
        incoming.status = Some("anulat".to_owned());
    }
    let mut tags: BTreeSet<String> = previous.tags.iter().cloned().collect();
    tags.extend(incoming.tags.iter().cloned());
    incoming.tags = tags.into_iter().collect();
}

fn validate_event(event: &Event, expected_year: i32) -> Result<()> {
    ensure!(
        !event.name.trim().is_empty(),
        "event name must not be empty"
    );
    let start = NaiveDate::parse_from_str(&event.d, "%Y-%m-%d")
        .with_context(|| format!("invalid start date for {}", event.name))?;
    ensure!(
        start.year() == expected_year,
        "event {} is outside expected year {expected_year}",
        event.name
    );
    if let Some(end) = &event.d2 {
        let end = NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .with_context(|| format!("invalid end date for {}", event.name))?;
        ensure!(end >= start, "event {} ends before it starts", event.name);
    }
    let url = Url::parse(&event.url).with_context(|| format!("invalid URL for {}", event.name))?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "unsafe URL scheme for {}",
        event.name
    );
    if let Some(status) = &event.status {
        ensure!(
            status == "anulat",
            "unknown status {status} for {}",
            event.name
        );
    }
    Ok(())
}

fn reject_duplicates(events: &[Event]) -> Result<()> {
    let mut identities = HashSet::new();
    for event in events {
        let identity = format!(
            "{}|{}|{}",
            event.d,
            normalize_space(&event.name).to_lowercase(),
            url_key(&event.url)
        );
        if !identities.insert(identity) {
            bail!("duplicate event record: {} ({})", event.name, event.d);
        }
    }
    Ok(())
}

fn url_key(raw: &str) -> String {
    if let Ok(mut url) = Url::parse(raw) {
        url.set_fragment(None);
        let normalized = url.to_string();
        return normalized.trim_end_matches('/').to_lowercase();
    }
    raw.trim().trim_end_matches('/').to_lowercase()
}

fn name_date_key(event: &Event) -> String {
    format!(
        "{}|{}",
        event.d,
        normalize_space(&event.name).to_lowercase()
    )
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("events.json");
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

fn prune_backups(directory: &Path, retain: usize) -> Result<()> {
    let mut backups = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("events-") && name.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(retain);
    for path in backups.into_iter().take(remove_count) {
        fs::remove_file(&path)
            .with_context(|| format!("could not prune backup {}", path.display()))?;
    }
    Ok(())
}

trait DateYear {
    fn year(&self) -> i32;
}

impl DateYear for NaiveDate {
    fn year(&self) -> i32 {
        chrono::Datelike::year(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/calendar.html");

    #[test]
    fn parses_supported_source_shapes() {
        let events = parse_events(FIXTURE, 2026).unwrap();
        assert_eq!(events.len(), 5);

        let cancelled = events
            .iter()
            .find(|event| event.name == "Făgăraș Rocks!")
            .unwrap();
        assert_eq!(cancelled.d, "2026-07-29");
        assert_eq!(cancelled.d2.as_deref(), Some("2026-08-02"));
        assert_eq!(cancelled.status.as_deref(), Some("anulat"));
        assert_eq!(cancelled.county, "BV");

        let ultra = events
            .iter()
            .find(|event| event.name == "Test Trail Ultra")
            .unwrap();
        assert!(ultra.tags.contains(&"ultra".to_owned()));
        assert_eq!(ultra.dist, "50 / 21 / 10 km");

        assert!(events.iter().all(|event| !event.name.contains("Moldova")));
        assert!(events.iter().all(|event| event.name != "Aleargă România"));
        let no_location = events
            .iter()
            .find(|event| event.name == "No Location Trail")
            .unwrap();
        assert_eq!(no_location.loc, "—");
        assert_eq!(no_location.county, "CJ");
        assert_eq!(no_location.dist, "12 / 5 km");
    }

    #[test]
    fn rejects_invalid_dates() {
        let invalid = "<p>• Februarie 31: <strong><a href=\"https://example.test\">Bad Trail</a></strong>, Brașov BV</p>";
        assert!(parse_events(invalid, 2026).is_err());
    }

    #[test]
    fn merge_retains_curated_events_and_fields() {
        let scraped = parse_events(FIXTURE, 2026).unwrap();
        let mut previous = scraped[0].clone();
        previous.note = Some("verificat manual".to_owned());
        previous.status = Some("anulat".to_owned());
        let manual = Event {
            d: "2026-06-01".to_owned(),
            d2: None,
            name: "Campionat manual".to_owned(),
            url: "https://fra.example/competition".to_owned(),
            loc: "Rășinari".to_owned(),
            county: "SB".to_owned(),
            dist: "vertical".to_owned(),
            tags: vec!["campionat".to_owned()],
            status: None,
            note: None,
            extra: BTreeMap::new(),
        };
        let baseline = Calendar {
            generated: "2026-01-01".to_owned(),
            sources: vec!["manual".to_owned()],
            events: vec![previous, manual],
            extra: BTreeMap::new(),
        };
        let policy = SafetyPolicy {
            minimum_scraped_events: 1,
            minimum_match_percent: 0,
            maximum_growth_percent: 500,
            force: false,
        };
        let (calendar, report) =
            merge_with_baseline(scraped, &baseline, 2026, &default_source(2026), &policy).unwrap();
        assert_eq!(report.retained, 1);
        assert_eq!(report.added, 4);
        assert!(
            calendar
                .events
                .iter()
                .any(|event| event.name == "Campionat manual")
        );
        let preserved = calendar
            .events
            .iter()
            .find(|event| event.note.is_some())
            .unwrap();
        assert_eq!(preserved.status.as_deref(), Some("anulat"));
    }

    #[test]
    fn safety_gate_rejects_unrelated_scrape() {
        let scraped = parse_events(FIXTURE, 2026).unwrap();
        let baseline = Calendar {
            generated: "2026-01-01".to_owned(),
            sources: vec![],
            events: vec![Event {
                d: "2026-01-01".to_owned(),
                d2: None,
                name: "Different".to_owned(),
                url: "https://different.example/event".to_owned(),
                loc: "Loc".to_owned(),
                county: "CJ".to_owned(),
                dist: "10 km".to_owned(),
                tags: vec![],
                status: None,
                note: None,
                extra: BTreeMap::new(),
            }],
            extra: BTreeMap::new(),
        };
        let policy = SafetyPolicy {
            minimum_scraped_events: 1,
            minimum_match_percent: 50,
            maximum_growth_percent: 25,
            force: false,
        };
        assert!(
            merge_with_baseline(scraped, &baseline, 2026, &default_source(2026), &policy).is_err()
        );
    }

    fn event(date: &str, name: &str, url: &str) -> Event {
        Event {
            d: date.to_owned(),
            d2: None,
            name: name.to_owned(),
            url: url.to_owned(),
            loc: "Loc".to_owned(),
            county: "CJ".to_owned(),
            dist: "10 km".to_owned(),
            tags: vec![],
            status: None,
            note: None,
            extra: BTreeMap::new(),
        }
    }

    fn calendar_of(events: Vec<Event>) -> Calendar {
        Calendar {
            generated: "2026-01-01".to_owned(),
            sources: vec![],
            events,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn source_url_follows_the_season() {
        assert!(default_source(2027).ends_with("alergare-2027/"));
        assert_ne!(default_source(2026), default_source(2027));
        assert_eq!(SUPPORTED_YEARS, &[2026, 2027]);
    }

    #[test]
    fn merge_carries_other_seasons_untouched() {
        let scraped = parse_events(FIXTURE, 2026).unwrap();
        let next_season = event("2027-05-15", "Sezon viitor", "https://example.test/2027");
        let baseline = calendar_of(vec![
            event("2026-03-14", "Vechi", "https://example.test/vechi"),
            next_season.clone(),
        ]);
        let policy = SafetyPolicy {
            minimum_scraped_events: 1,
            minimum_match_percent: 0,
            maximum_growth_percent: 500,
            force: false,
        };
        let (calendar, report) =
            merge_with_baseline(scraped, &baseline, 2026, &default_source(2026), &policy).unwrap();
        assert_eq!(report.carried, 1);
        assert_eq!(report.retained, 1);
        assert!(calendar.events.contains(&next_season));
    }

    #[test]
    fn merge_accepts_a_season_the_calendar_does_not_cover_yet() {
        let scraped = parse_events(FIXTURE, 2027).unwrap();
        let baseline = calendar_of(vec![event(
            "2026-03-14",
            "Vechi",
            "https://example.test/vechi",
        )]);
        // The gates a full season is held to must not reject the first scrape of a new one.
        let policy = SafetyPolicy {
            minimum_scraped_events: 50,
            minimum_match_percent: 50,
            maximum_growth_percent: 25,
            force: false,
        };
        let (calendar, report) =
            merge_with_baseline(scraped, &baseline, 2027, &default_source(2027), &policy).unwrap();
        assert_eq!(report.added, 5);
        assert_eq!(report.carried, 1);
        assert_eq!(calendar.events.len(), 6);
        assert!(
            calendar
                .events
                .iter()
                .any(|stored| event_year(stored) == Some(2026))
        );
        assert_eq!(
            calendar
                .events
                .iter()
                .filter(|stored| event_year(stored) == Some(2027))
                .count(),
            5
        );
    }

    fn lenient_policy() -> SafetyPolicy {
        SafetyPolicy {
            minimum_scraped_events: 1,
            minimum_match_percent: 0,
            maximum_growth_percent: 5000,
            force: false,
        }
    }

    #[test]
    fn refresh_skips_a_season_the_source_has_not_published() {
        let baseline = calendar_of(vec![event(
            "2026-03-14",
            "Vechi",
            "https://example.test/vechi",
        )]);
        let (calendar, summary) =
            refresh_seasons(&baseline, &[2026, 2027], &lenient_policy(), |year| {
                if year == 2027 {
                    bail!("source returned an unsuccessful status: 404");
                }
                Ok((default_source(year), FIXTURE.to_owned()))
            })
            .unwrap();

        assert_eq!(summary.refreshed.len(), 1);
        assert_eq!(summary.refreshed[0].0, 2026);
        assert_eq!(summary.skipped.len(), 1);
        assert_eq!(summary.skipped[0].0, 2027);
        assert!(summary.skipped[0].1.contains("404"));
        assert!(calendar.events.iter().any(|stored| stored.name == "Vechi"));
        assert!(
            calendar
                .events
                .iter()
                .all(|stored| event_year(stored) == Some(2026))
        );
    }

    #[test]
    fn refresh_aborts_when_a_covered_season_fails() {
        let baseline = calendar_of(vec![
            event("2026-03-14", "Vechi", "https://example.test/vechi"),
            event("2027-05-15", "Sezon viitor", "https://example.test/2027"),
        ]);
        let failure = refresh_seasons(&baseline, &[2026, 2027], &lenient_policy(), |year| {
            if year == 2027 {
                bail!("source request failed");
            }
            Ok((default_source(year), FIXTURE.to_owned()))
        })
        .unwrap_err();
        assert!(format!("{failure:#}").contains("season 2027 could not be refreshed"));
    }

    #[test]
    fn refresh_updates_every_configured_season() {
        let baseline = calendar_of(vec![event(
            "2026-03-14",
            "Vechi",
            "https://example.test/vechi",
        )]);
        let (calendar, summary) =
            refresh_seasons(&baseline, &[2027, 2026, 2027], &lenient_policy(), |year| {
                Ok((default_source(year), FIXTURE.to_owned()))
            })
            .unwrap();

        assert_eq!(
            summary
                .refreshed
                .iter()
                .map(|(year, _)| *year)
                .collect::<Vec<_>>(),
            vec![2026, 2027]
        );
        assert!(summary.skipped.is_empty());
        let season = |year| {
            calendar
                .events
                .iter()
                .filter(|stored| event_year(stored) == Some(year))
                .count()
        };
        // 2026 keeps its curated event alongside the five scraped ones; 2027 starts from them.
        assert_eq!(season(2026), 6);
        assert_eq!(season(2027), 5);
        assert_eq!(calendar.sources.len(), 2);
    }

    #[test]
    fn merge_rejects_events_from_another_season() {
        let scraped = parse_events(FIXTURE, 2026).unwrap();
        let baseline = calendar_of(vec![event(
            "2027-05-15",
            "Sezon viitor",
            "https://example.test/2027",
        )]);
        let policy = SafetyPolicy {
            minimum_scraped_events: 1,
            minimum_match_percent: 0,
            maximum_growth_percent: 500,
            force: false,
        };
        assert!(
            merge_with_baseline(scraped, &baseline, 2027, &default_source(2027), &policy).is_err()
        );
    }
}
