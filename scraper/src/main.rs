use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use montana_scraper::{
    SUPPORTED_YEARS, SafetyPolicy, default_source, fetch_html, read_calendar, refresh_seasons,
    write_calendar_atomic,
};

#[derive(Debug, Parser)]
#[command(about = "Safely refresh calendartrail.ro event data", version)]
struct Args {
    /// HTML source URL for a single --year, HTTPS only. Defaults to each season's page.
    #[arg(long)]
    source: Option<String>,

    /// Parse a local HTML fixture instead of fetching the source.
    #[arg(long)]
    fixture: Option<PathBuf>,

    /// Existing and destination calendar JSON.
    #[arg(long, default_value = "app/events.json")]
    output: PathBuf,

    /// Directory in which last-known-good snapshots are retained.
    #[arg(long)]
    backup_dir: Option<PathBuf>,

    /// Calendar seasons to refresh. Repeat the flag for more than one.
    #[arg(long = "year", num_args = 1.., default_values_t = SUPPORTED_YEARS.to_vec())]
    years: Vec<i32>,

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
    let mut years = args.years.clone();
    years.sort_unstable();
    years.dedup();
    ensure!(!years.is_empty(), "at least one --year is required");
    ensure!(
        args.source.is_none() || years.len() == 1,
        "--source refreshes a single --year"
    );
    ensure!(
        args.fixture.is_none() || years.len() == 1,
        "--fixture refreshes a single --year"
    );

    let policy = SafetyPolicy {
        minimum_scraped_events: args.minimum_events,
        minimum_match_percent: args.minimum_match_percent,
        maximum_growth_percent: args.maximum_growth_percent,
        force: args.force,
    };
    let baseline = read_calendar(&args.output)?;
    let (calendar, summary) = refresh_seasons(&baseline, &years, &policy, |year| {
        let source = args.source.clone().unwrap_or_else(|| default_source(year));
        let html = if let Some(fixture) = &args.fixture {
            fs::read_to_string(fixture)
                .with_context(|| format!("could not read fixture {}", fixture.display()))?
        } else {
            fetch_html(&source)?
        };
        Ok((source, html))
    })?;

    for (year, report) in &summary.refreshed {
        eprintln!(
            "{year}: scraped={}, matched={}, changed={}, added={}, retained={}, carried={}",
            report.scraped,
            report.matched,
            report.changed,
            report.added,
            report.retained,
            report.carried
        );
    }
    for (year, reason) in &summary.skipped {
        eprintln!("{year}: skipped, {reason}");
    }
    eprintln!("final={}", calendar.events.len());

    if args.dry_run {
        if args.print_json {
            println!("{}", serde_json::to_string_pretty(&calendar)?);
        } else {
            println!(
                "validated candidate with {} events across {} season(s); no files changed",
                calendar.events.len(),
                summary.refreshed.len()
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
