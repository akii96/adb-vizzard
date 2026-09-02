//! Manual end-to-end check against a real workspace.
//!
//! Runs the same code path the app uses — resolve, list children, discover the
//! harness, fetch and parse — and prints a summary. Credentials come from `.env`
//! or the environment; nothing is written anywhere.
//!
//! ```text
//! cargo run --example live_load --manifest-path src-tauri/Cargo.toml -- <run_id>
//! ```
//!
//! Kept as an example rather than a test because it needs credentials and network,
//! and CI has neither.

use std::sync::Arc;

use adb_vizzard::cache::ArtifactCache;
use adb_vizzard::client::AdbClient;
use adb_vizzard::comparator::{build_table, Aggregation};
use adb_vizzard::creds::SessionCreds;
use adb_vizzard::fetcher::{ProgressSink, RemoteSource};
use adb_vizzard::model::Side;
use tokio_util::sync::CancellationToken;

fn load_env() -> Option<(String, String)> {
    if let (Ok(host), Ok(token)) = (
        std::env::var("DATABRICKS_HOST"),
        std::env::var("DATABRICKS_TOKEN"),
    ) {
        return Some((host, token));
    }

    // Walk up from the manifest directory to find a .env.
    for dir in [".", ".."] {
        let path = std::path::Path::new(dir).join(".env");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut host = None;
            let mut token = None;
            for line in text.lines() {
                let line = line.trim();
                if let Some((key, value)) = line.split_once('=') {
                    let value = value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();
                    match key.trim() {
                        "DATABRICKS_HOST" => host = Some(value),
                        "DATABRICKS_TOKEN" => token = Some(value),
                        _ => {}
                    }
                }
            }
            if let (Some(h), Some(t)) = (host, token) {
                return Some((h, t));
            }
        }
    }
    None
}

#[tokio::main]
async fn main() {
    let run_id = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: live_load <run_id_or_url>");
        std::process::exit(2);
    });

    let (host, token) = load_env().unwrap_or_else(|| {
        eprintln!("no credentials: set DATABRICKS_HOST/DATABRICKS_TOKEN or provide a .env");
        std::process::exit(2);
    });

    let creds = Arc::new(SessionCreds::new(&host, &token).expect("credentials"));
    let http = adb_vizzard::client::build_http_client().expect("http client");
    let client = Arc::new(AdbClient::new(http, Arc::clone(&creds)));

    match client.verify().await {
        Ok(user) => println!(
            "connected to {host} as {}",
            user.unwrap_or("(unknown)".into())
        ),
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(1);
        }
    }

    let run_id = adb_vizzard::runref::extract_run_id(&run_id).expect("run id");
    let transport = client.probe_transport(&run_id).await;
    println!("transport: {transport:?}");

    let cache = Arc::new(ArtifactCache::new(64));
    let source = RemoteSource::new(Arc::clone(&client), Arc::clone(&cache), transport, 8, 24);

    let sink: ProgressSink = Arc::new(|_| {});
    let cancel = CancellationToken::new();

    let started = std::time::Instant::now();
    let side = match source.load(Side::A, &run_id, &sink, &cancel).await {
        Ok(side) => side,
        Err(e) => {
            eprintln!("load failed: {e}");
            std::process::exit(1);
        }
    };

    println!();
    println!("run      : {} ({})", side.run_name, side.run_id);
    println!("label    : {}", side.label);
    println!(
        "children : {} ok, {} failed",
        side.children.len(),
        side.failures.len()
    );
    println!("elapsed  : {} ms (cold)", started.elapsed().as_millis());
    for failure in &side.failures {
        println!("  FAILED {}: {}", failure.run_name, failure.reason);
    }

    println!();
    println!(
        "{:<20} {:>6} {:>12} {:>12} {:>12}",
        "group", "conc", "out tok/s", "tpot ms", "ttft ms"
    );
    let table = build_table(
        &side,
        None,
        &[],
        Aggregation::Median,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    for row in &table.rows {
        if let Some(cell) = &row.a {
            println!(
                "{:<20} {:>6} {:>12.2} {:>12.2} {:>12.2}",
                row.group,
                row.concurrency,
                cell.metrics.output_throughput,
                cell.metrics.median_tpot_ms,
                cell.metrics.median_ttft_ms
            );
        }
    }

    println!();
    println!(
        "artifacts held in memory after cold load: {:?}",
        cache.stats()
    );

    // Second pass exercises the in-memory session cache. Report a failure
    // distinctly from a cache miss — conflating them hides rate limiting.
    let warm = std::time::Instant::now();
    match source.load(Side::A, &run_id, &sink, &cancel).await {
        Ok(second) => println!(
            "warm reload: {} ms, from_cache={}, children={}",
            warm.elapsed().as_millis(),
            second.from_cache,
            second.children.len()
        ),
        Err(e) => println!(
            "warm reload FAILED after {} ms: {e}",
            warm.elapsed().as_millis()
        ),
    }
}
