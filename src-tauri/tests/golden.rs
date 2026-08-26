//! Golden-file tests against a real `adb-pull` tree.
//!
//! These run offline through local-folder mode, so the parsers, the
//! `(group, concurrency)` join and both exporters are verified without any
//! credentials or network. Expected values are taken from the fixture artifacts
//! in `fixtures/`, rounded exactly the way `adb-summarize` rounds them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adb_vizzard::comparator::{build_table, Aggregation};
use adb_vizzard::exporter::{
    write_comparison_csv, write_comparison_xlsx, write_raw_csv, ExportOptions,
};
use adb_vizzard::fetcher::LocalDirSource;
use adb_vizzard::model::{Side, SideData};

const PULL_DIR: &str = "exp_pull_20260325_103811";
const AMD_PARENT: &str = "AMD-GPT-TP1-vllm-private-rocm721-03252026";
const OAI_PARENT: &str = "OAI-GPT-TP1-vllm-private-rocm721-03252026";

/// A sweep from the other benchmark harness: `sglang.bench_serving` rather than
/// `vllm bench serve`. Different artifact directory, different metric spellings,
/// and a bare `Infinity` literal in the JSON.
const SGLANG_PULL_DIR: &str = "exp_pull_sglang_oob_vllm_tp4_nvfp4";
const SGLANG_PARENT: &str = "oob_vllm_tp4_nvfp4";

fn fixtures_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is src-tauri/, so fixtures/ is one level up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri should have a parent directory")
        .join("fixtures")
}

fn fixtures_root() -> PathBuf {
    fixtures_dir().join(PULL_DIR)
}

/// Loads one parent directory, discarding progress events.
fn load(parent: &str) -> SideData {
    let sink: adb_vizzard::fetcher::ProgressSink = Arc::new(|_| {});
    LocalDirSource::new(fixtures_root().join(parent))
        .load(Side::A, &sink)
        .unwrap_or_else(|err| panic!("failed to load {parent}: {err}"))
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("adb-vizzard-golden-{}-{name}", std::process::id()))
}

#[test]
fn fixture_tree_is_present() {
    let root = fixtures_root();
    assert!(
        root.is_dir(),
        "fixture tree missing at {} — see fixtures/README.md",
        root.display()
    );
}

#[test]
fn discovers_parents_without_the_parent_prefix() {
    // This pull's parent directories were renamed by hand, so detection has to be
    // structural. A prefix-based implementation would find nothing here.
    let parents = LocalDirSource::list_parent_dirs(&fixtures_root());
    let names: Vec<String> = parents
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
        .collect();

    assert_eq!(names, vec![AMD_PARENT.to_string(), OAI_PARENT.to_string()]);
    assert!(!names.iter().any(|n| n.starts_with("parent_")));
}

#[test]
fn pointing_at_the_pull_root_resolves_a_parent() {
    // Users pick either the pull root or one parent; both must work.
    let sink: adb_vizzard::fetcher::ProgressSink = Arc::new(|_| {});
    let data = LocalDirSource::new(fixtures_root())
        .load(Side::A, &sink)
        .expect("pull root should resolve to its first parent");
    assert_eq!(data.children.len(), 1);
}

#[test]
fn parses_the_amd_child_exactly() {
    let data = load(AMD_PARENT);

    assert_eq!(data.run_name, AMD_PARENT);
    assert_eq!(data.children.len(), 1);

    let child = &data.children[0];
    // child_unleashed-stork-87_ccf846e0 -> the prefix and 8-hex suffix are stripped.
    assert_eq!(child.run_name, "unleashed-stork-87");
    assert_eq!(child.group, "in1000_out100");
    assert_eq!(child.input_len, 1000);
    assert_eq!(child.output_len, 100);
    assert_eq!(child.concurrency, 16);

    // Raw values from the fixture, rounded to two decimals as the CLI does:
    //   median_itl_ms            29.49584199814126   -> 29.5
    //   median_ttft_ms         1225.3003310179338    -> 1225.3
    //   median_tpot_ms           29.672150500118732  -> 29.67
    //   median_e2el_ms         4161.693745001685     -> 4161.69
    //   output_throughput       384.02979945866105   -> 384.03
    //   total_token_throughput 4224.327794045272     -> 4224.33
    assert_eq!(child.metrics.median_itl_ms, 29.5);
    assert_eq!(child.metrics.median_ttft_ms, 1225.3);
    assert_eq!(child.metrics.median_tpot_ms, 29.67);
    assert_eq!(child.metrics.median_e2el_ms, 4161.69);
    assert_eq!(child.metrics.output_throughput, 384.03);
    assert_eq!(child.metrics.total_token_throughput, 4224.33);

    assert_eq!(child.metadata.flag("tensor_parallel_size").unwrap(), "1");
    assert_eq!(
        child.metadata.flag("model").unwrap(),
        "amd/gpt-oss-120b-w-mxfp4-a-fp8"
    );
    assert_eq!(
        child
            .metadata
            .env_vars
            .get("VLLM_ROCM_USE_AITER")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        child
            .metadata
            .env_vars
            .get("HIP_VISIBLE_DEVICES")
            .map(String::as_str),
        Some("6")
    );
}

#[test]
fn parses_the_oai_child_exactly() {
    let data = load(OAI_PARENT);
    let child = &data.children[0];

    assert_eq!(child.run_name, "clumsy-fox-596");
    assert_eq!(child.group, "in1000_out100");
    assert_eq!(child.concurrency, 16);

    //   median_itl_ms            27.593816514126956  -> 27.59
    //   median_ttft_ms          432.0848010247573    -> 432.08
    //   median_tpot_ms           27.67487358586449   -> 27.67
    //   median_e2el_ms         3171.469761058688     -> 3171.47
    //   output_throughput       503.81609200748386   -> 503.82
    //   total_token_throughput 5541.977012082322     -> 5541.98
    assert_eq!(child.metrics.median_itl_ms, 27.59);
    assert_eq!(child.metrics.median_ttft_ms, 432.08);
    assert_eq!(child.metrics.median_tpot_ms, 27.67);
    assert_eq!(child.metrics.median_e2el_ms, 3171.47);
    assert_eq!(child.metrics.output_throughput, 503.82);
    assert_eq!(child.metrics.total_token_throughput, 5541.98);

    assert_eq!(child.metadata.flag("model").unwrap(), "openai/gpt-oss-120b");
    assert_eq!(
        child
            .metadata
            .env_vars
            .get("HIP_VISIBLE_DEVICES")
            .map(String::as_str),
        Some("7")
    );
}

#[test]
fn the_scrubbed_fixture_carries_no_live_token() {
    // A real commands.txt embeds HF_TOKEN. If someone refreshes the fixture from a
    // live pull without scrubbing, this fails before the secret spreads further.
    for parent in [AMD_PARENT, OAI_PARENT] {
        let data = load(parent);
        let token = data.children[0]
            .metadata
            .env_vars
            .get("HF_TOKEN")
            .map(String::as_str)
            .unwrap_or_default();
        assert_eq!(
            token, "hf_REDACTED_FOR_FIXTURE",
            "{parent} fixture has an unscrubbed HF_TOKEN — see fixtures/README.md"
        );
    }
}

#[test]
fn compares_the_two_parents_on_group_and_concurrency() {
    let amd = load(AMD_PARENT);
    let oai = load(OAI_PARENT);

    let fields = vec![
        "tensor_parallel_size".to_string(),
        "env:VLLM_ROCM_USE_AITER".to_string(),
        "env:NOT_SET_ANYWHERE".to_string(),
    ];
    let table = build_table(&amd, Some(&oai), &fields, Aggregation::Median);

    // Both children are in1000_out100 at concurrency 16, so they join into one row.
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.matched_rows, 1);
    assert_eq!(table.a_only_rows, 0);
    assert_eq!(table.b_only_rows, 0);

    let row = &table.rows[0];
    assert_eq!(row.group, "in1000_out100");
    assert_eq!(row.concurrency, 16);

    // 384.03 / 503.82 * 100 = 76.2%: the AMD build is slower on throughput here.
    let throughput_ratio = row.ratios.get("output_throughput").copied().unwrap();
    assert!(
        (throughput_ratio - 76.22).abs() < 0.01,
        "unexpected throughput ratio: {throughput_ratio}"
    );

    // 29.67 / 27.67 * 100 = 107.2%: and marginally worse on TPOT.
    let tpot_ratio = row.ratios.get("median_tpot_ms").copied().unwrap();
    assert!(
        (tpot_ratio - 107.23).abs() < 0.01,
        "unexpected TPOT ratio: {tpot_ratio}"
    );

    // A field absent from commands.txt must read `image-default`, not blank.
    assert_eq!(
        table.field_headers,
        vec![
            "tensor_parallel_size",
            "VLLM_ROCM_USE_AITER",
            "NOT_SET_ANYWHERE"
        ]
    );
    let cell = row.a.as_ref().unwrap();
    assert_eq!(
        cell.fields.get("tensor_parallel_size").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        cell.fields.get("VLLM_ROCM_USE_AITER").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        cell.fields.get("NOT_SET_ANYWHERE").map(String::as_str),
        Some("image-default")
    );
}

/// Loads the sglang sweep, which exercises the second harness end to end.
fn load_sglang() -> SideData {
    let sink: adb_vizzard::fetcher::ProgressSink = Arc::new(|_| {});
    LocalDirSource::new(fixtures_dir().join(SGLANG_PULL_DIR).join(SGLANG_PARENT))
        .load(Side::B, &sink)
        .expect("sglang fixture should load")
}

// This is the regression test for the bug that made every sglang sweep fail:
// the benchmark path was hardcoded to `0_vllm_bench_serve`, the metric names
// differ, and the JSON contains a bare `Infinity` that serde_json rejects.
#[test]
fn parses_a_sglang_harness_run() {
    let data = load_sglang();
    assert_eq!(data.children.len(), 1);

    let child = &data.children[0];
    assert_eq!(child.run_name, "silent-dove-115");
    assert_eq!(child.input_len, 128000);
    assert_eq!(child.output_len, 1024);
    assert_eq!(child.group, "in128000_out1024");
    assert_eq!(child.concurrency, 256);

    // Read through the sglang aliases: median_e2e_latency_ms and total_throughput.
    assert_eq!(child.metrics.median_itl_ms, 17.12);
    assert_eq!(child.metrics.median_ttft_ms, 505895.77);
    assert_eq!(child.metrics.median_tpot_ms, 98.67);
    assert_eq!(child.metrics.median_e2el_ms, 594877.43);
    assert_eq!(child.metrics.output_throughput, 392.69);
    assert_eq!(child.metrics.total_token_throughput, 49491.3);

    // The artifact path recorded for export must be the harness that was found,
    // not the hardcoded default.
    assert!(
        child.benchmark_path.contains("0_sglang_bench_serve"),
        "expected the sglang harness path, got {}",
        child.benchmark_path
    );

    assert_eq!(child.metadata.flag("tensor_parallel_size").unwrap(), "4");
}

#[test]
fn the_sglang_fixture_carries_no_live_token() {
    let data = load_sglang();
    let token = data.children[0]
        .metadata
        .env_vars
        .get("HF_TOKEN")
        .map(String::as_str)
        .unwrap_or_default();
    assert_eq!(
        token, "hf_REDACTED_FOR_FIXTURE",
        "sglang fixture has an unscrubbed HF_TOKEN — see fixtures/README.md"
    );
}

// Both harnesses normalise to the same shape, so a vllm sweep and an sglang sweep
// can be compared against each other.
#[test]
fn compares_across_the_two_harnesses() {
    let vllm = load(AMD_PARENT);
    let sglang = load_sglang();

    let table = build_table(&vllm, Some(&sglang), &[], Aggregation::Median);

    // Different input/output lengths, so nothing joins; both sides still appear.
    assert_eq!(table.matched_rows, 0);
    assert_eq!(table.a_only_rows, 1);
    assert_eq!(table.b_only_rows, 1);
    assert_eq!(table.rows.len(), 2);
}

#[test]
fn exports_the_fixture_comparison_to_csv() {
    let amd = load(AMD_PARENT);
    let oai = load(OAI_PARENT);
    let fields = vec!["tensor_parallel_size".to_string()];
    let table = build_table(&amd, Some(&oai), &fields, Aggregation::Median);

    let path = temp_path("compare.csv");
    write_comparison_csv(&path, &table, &ExportOptions::default()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();

    // Two header rows, then one data row.
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("random_input_len,random_output_len,max_concurrency,"));
    assert!(lines[0].contains("A vs B"));
    assert!(lines[1].starts_with(",,,"));

    // The metric values land in the data row exactly as parsed.
    assert!(lines[2].starts_with("1000,100,16,"));
    assert!(
        lines[2].contains("384.03"),
        "side A throughput missing: {}",
        lines[2]
    );
    assert!(
        lines[2].contains("503.82"),
        "side B throughput missing: {}",
        lines[2]
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn exports_the_fixture_comparison_to_xlsx_and_raw_csv() {
    let amd = load(AMD_PARENT);
    let oai = load(OAI_PARENT);
    let table = build_table(&amd, Some(&oai), &[], Aggregation::Median);

    let xlsx = temp_path("compare.xlsx");
    write_comparison_xlsx(&xlsx, &table, &ExportOptions::default()).unwrap();
    let bytes = std::fs::read(&xlsx).unwrap();
    assert_eq!(&bytes[..2], b"PK", "xlsx should be a zip archive");
    let _ = std::fs::remove_file(xlsx);

    let raw = temp_path("raw.csv");
    write_raw_csv(&raw, &[("A", &amd), ("B", &oai)]).unwrap();
    let text = std::fs::read_to_string(&raw).unwrap();
    let lines: Vec<&str> = text.lines().collect();

    // Header plus one row per child, carrying the source path for traceability.
    assert_eq!(lines.len(), 3);
    assert!(lines[0].ends_with("benchmark_yaml_path"));
    assert!(lines[1].starts_with("A,"));
    assert!(lines[2].starts_with("B,"));
    assert!(lines[1].contains("benchmark_results"));

    let _ = std::fs::remove_file(raw);
}
