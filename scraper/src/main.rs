use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use montana_scraper::{
    DEFAULT_SOURCE, SafetyPolicy, fetch_html, merge_with_baseline, parse_events, read_calendar,
    write_calendar_atomic,
};

#[derive(Debug, Parser)]
#[command(about = "Safely refresh calendartrail.ro event data", version)]
struct Args {
    /// HTML source URL. HTTPS is required unless --fixture is used.
    #[arg(long, default_value = DEFAULT_SOURCE)]
    source: String,

    /// Parse a local HTML fixture instead of fetching the source.
    #[arg(long)]
    fixture: Option<PathBuf>,

    /// Existing and destination calendar JSON.
    #[arg(long, default_value = "app/events.json")]
    output: PathBuf,

    /// Directory in which last-known-good snapshots are retained.
    #[arg(long)]
    backup_dir: Option<PathBuf>,

    /// Calendar year represented by the source page.
    #[arg(long, default_value_t = 2026)]
    year: i32,

    /// Refuse a source page yielding fewer events.
    #[arg(long, default_value_t = 50)]
    minimum_events: usize,

    /// Refuse a scrape matching less of the existing calendar.
    #[arg(long, default_value_t = 50)]
    minimum_match_percent: u8,

    /// Refuse a scrape that adds more than this percentage at once.
    #[arg(long, default_value_t = 25)]
    maximum_growth_percent: u16,

    /// Print the merged calendar without changing files.
    #[arg(long)]
    dry_run: bool,

    /// Print candidate JSON during a dry run.
    #[arg(long, requires = "dry_run")]
    print_json: bool,

    /// Bypass count and drift gates. Schema and URL checks remain active.
    #[arg(long)]
    force: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("scrape failed: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let baseline = read_calendar(&args.output)?;
    let html = if let Some(fixture) = &args.fixture {
        fs::read_to_string(fixture)
            .with_context(|| format!("could not read fixture {}", fixture.display()))?
    } else {
        fetch_html(&args.source)?
    };
    let scraped = parse_events(&html, args.year)?;
    let policy = SafetyPolicy {
        minimum_scraped_events: args.minimum_events,
        minimum_match_percent: args.minimum_match_percent,
        maximum_growth_percent: args.maximum_growth_percent,
        force: args.force,
    };
    let (calendar, report) = merge_with_baseline(scraped, &baseline, &args.source, &policy)?;

    eprintln!(
        "scraped={}, matched={}, changed={}, added={}, retained={}, final={}",
        report.scraped,
        report.matched,
        report.changed,
        report.added,
        report.retained,
        calendar.events.len()
    );
    if args.dry_run {
        if args.print_json {
            println!("{}", serde_json::to_string_pretty(&calendar)?);
        } else {
            println!(
                "validated candidate with {} events; no files changed",
                calendar.events.len()
            );
        }
    } else {
        write_calendar_atomic(&args.output, &calendar, args.backup_dir.as_deref())?;
        println!(
            "updated {} with {} events",
            args.output.display(),
            calendar.events.len()
        );
    }
    Ok(())
}
