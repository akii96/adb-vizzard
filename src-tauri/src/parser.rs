//! Parsers for the two artifacts a child run contributes.
//!
//! Semantics deliberately mirror `adb_tools/summarize.py` so this port produces
//! the same numbers as the CLI people already trust. The golden-file tests in
//! `tests/` assert that against real pulls.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Written when a requested comparison field is absent, matching the CLI. The
/// distinction between "not set" and "inherited from the image" is meaningful,
/// so this is not an empty string.
pub const IMAGE_DEFAULT: &str = "image-default";

/// The six metric columns, in the order the example CSV uses.
pub const METRIC_KEYS: [&str; 6] = [
    "median_itl_ms",
    "median_ttft_ms",
    "median_tpot_ms",
    "median_e2el_ms",
    "output_throughput",
    "total_token_throughput",
];

fn env_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^-e\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)=(?P<value>".*"|'.*'|[^\s]+)$"#)
            .unwrap()
    })
}

fn flag_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^--(?P<name>[\w-]+)(?:\s+(?P<value>.+))?$").unwrap())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkMetrics {
    pub median_itl_ms: f64,
    pub median_ttft_ms: f64,
    pub median_tpot_ms: f64,
    pub median_e2el_ms: f64,
    /// True when `median_e2el_ms` was derived rather than read, so the UI can
    /// mark the number as an estimate. See [`approximate_e2el`].
    pub e2el_approximate: bool,
    pub output_throughput: f64,
    pub total_token_throughput: f64,
}

impl BenchmarkMetrics {
    pub fn get(&self, key: &str) -> Option<f64> {
        match key {
            "median_itl_ms" => Some(self.median_itl_ms),
            "median_ttft_ms" => Some(self.median_ttft_ms),
            "median_tpot_ms" => Some(self.median_tpot_ms),
            "median_e2el_ms" => Some(self.median_e2el_ms),
            "output_throughput" => Some(self.output_throughput),
            "total_token_throughput" => Some(self.total_token_throughput),
            _ => None,
        }
    }
}

/// Flags and env vars lifted out of `commands.txt`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CommandMetadata {
    /// A flag can legitimately appear more than once (the serve command and the
    /// bench command both pass `--model`), so values are collected as a set and
    /// only treated as ambiguous when they actually disagree.
    pub flags: BTreeMap<String, BTreeSet<String>>,
    pub env_vars: BTreeMap<String, String>,
    /// Container image, read from the `# Image:` header comment.
    ///
    /// It appears in the file only as a positional argument to `docker run`, which
    /// the flag parser cannot see, and the header states it explicitly.
    pub image: Option<String>,
}

impl CommandMetadata {
    /// Returns the single value for a flag, or an error if missing or ambiguous.
    pub fn flag(&self, name: &str) -> AppResult<&str> {
        let normalized = normalize_flag_name(name);
        let values = self
            .flags
            .get(&normalized)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                AppError::parse(format!("missing --{} in commands.txt", dashed(name)))
            })?;

        if values.len() > 1 {
            let listed: Vec<&str> = values.iter().map(String::as_str).collect();
            return Err(AppError::parse(format!(
                "ambiguous --{} values in commands.txt: {}",
                dashed(name),
                listed.join(", ")
            )));
        }

        Ok(values.iter().next().map(String::as_str).unwrap_or_default())
    }

    pub fn flag_i64(&self, name: &str) -> AppResult<i64> {
        let raw = self.flag(name)?;
        raw.parse::<i64>().map_err(|_| {
            AppError::parse(format!(
                "expected an integer for --{}, got {raw:?}",
                dashed(name)
            ))
        })
    }

    /// Resolves a `--compare-fields` entry. `env:NAME` reads a docker env var;
    /// anything else reads a flag. A missing value yields [`IMAGE_DEFAULT`].
    pub fn compare_value(&self, field: &str) -> AppResult<String> {
        if let Some(env_name) = field.strip_prefix("env:") {
            return Ok(self
                .env_vars
                .get(env_name)
                .cloned()
                .unwrap_or_else(|| IMAGE_DEFAULT.to_string()));
        }

        let normalized = normalize_flag_name(field);
        match self.flags.get(&normalized) {
            None => Ok(IMAGE_DEFAULT.to_string()),
            Some(values) if values.is_empty() => Ok(IMAGE_DEFAULT.to_string()),
            Some(values) if values.len() > 1 => {
                let listed: Vec<&str> = values.iter().map(String::as_str).collect();
                Err(AppError::parse(format!(
                    "ambiguous --{} values in commands.txt: {}",
                    dashed(field),
                    listed.join(", ")
                )))
            }
            Some(values) => Ok(values.iter().next().cloned().unwrap_or_default()),
        }
    }
}

/// Column header for a comparison field: `env:FOO` displays as `FOO`.
pub fn comparison_header(field: &str) -> &str {
    field.strip_prefix("env:").unwrap_or(field)
}

fn normalize_flag_name(name: &str) -> String {
    name.trim().replace('-', "_")
}

fn dashed(name: &str) -> String {
    name.replace('_', "-")
}

fn strip_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == last && (first == b'"' || first == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// Parses `commands.txt` into flags and env vars.
///
/// Lines are trimmed, comments skipped, and a trailing shell line-continuation
/// backslash removed before matching, which is what makes the indented
/// `-e VAR="value" \` lines inside a `docker run` invocation match.
pub fn parse_commands(text: &str) -> CommandMetadata {
    let mut meta = CommandMetadata::default();

    for raw_line in text.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            if let Some(image) = line.strip_prefix("# Image:") {
                let image = image.trim();
                if !image.is_empty() {
                    meta.image = Some(image.to_string());
                }
            }
            continue;
        }
        if let Some(stripped) = line.strip_suffix('\\') {
            line = stripped.trim_end();
        }

        if let Some(caps) = env_line_re().captures(line) {
            let name = caps.name("name").map(|m| m.as_str()).unwrap_or_default();
            let value = caps.name("value").map(|m| m.as_str()).unwrap_or_default();
            meta.env_vars
                .insert(name.to_string(), strip_quotes(value).to_string());
            continue;
        }

        if let Some(caps) = flag_line_re().captures(line) {
            let name =
                normalize_flag_name(caps.name("name").map(|m| m.as_str()).unwrap_or_default());
            // A bare flag is a boolean switch, recorded as "true" like the CLI.
            let value = match caps.name("value") {
                Some(m) => strip_quotes(m.as_str().trim()).to_string(),
                None => "true".to_string(),
            };
            meta.flags.entry(name).or_default().insert(value);
        }
    }

    meta
}

/// Accepted spellings for each metric, in preference order.
///
/// Two benchmark harnesses produce this artifact and they disagree on two names:
/// `vllm bench serve` writes `median_e2el_ms` and `total_token_throughput`, while
/// `sglang.bench_serving` writes `median_e2e_latency_ms` and `total_throughput`.
/// Same quantities, different labels.
const METRIC_ALIASES: [(&str, &[&str]); 6] = [
    ("median_itl_ms", &["median_itl_ms"]),
    ("median_ttft_ms", &["median_ttft_ms"]),
    ("median_tpot_ms", &["median_tpot_ms"]),
    (
        "median_e2el_ms",
        &["median_e2el_ms", "median_e2e_latency_ms"],
    ),
    ("output_throughput", &["output_throughput"]),
    (
        "total_token_throughput",
        &["total_token_throughput", "total_throughput"],
    ),
];

fn bare_constant_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Only after a delimiter, so the same word inside a string value is untouched.
    RE.get_or_init(|| Regex::new(r"([:\[,]\s*)(-?Infinity|-?NaN)(\s*[,\}\]])").unwrap())
}

/// Rewrites non-standard JSON literals to `null`.
///
/// `sglang.bench_serving` writes `"request_rate": Infinity`, which Python's `json`
/// accepts as an extension but is invalid per RFC 8259, so `serde_json` rejects the
/// whole document. None of the fields we read are ever infinite, so mapping the
/// literal to `null` costs nothing and keeps the file parseable.
fn sanitize_json_constants(text: &str) -> std::borrow::Cow<'_, str> {
    // Cheap check first: the substitution regex is not free, and most files are
    // already valid.
    if !text.contains("Infinity") && !text.contains("NaN") {
        return std::borrow::Cow::Borrowed(text);
    }

    let mut out = text.to_string();
    // A run of `[NaN, NaN]` shares delimiters between matches, so repeat until
    // the text stops changing.
    loop {
        let replaced = bare_constant_re()
            .replace_all(&out, "${1}null${3}")
            .to_string();
        if replaced == out {
            break;
        }
        out = replaced;
    }
    std::borrow::Cow::Owned(out)
}

fn parse_benchmark_json(text: &str) -> AppResult<serde_json::Value> {
    let sanitized = sanitize_json_constants(text);
    serde_json::from_str(&sanitized)
        .map_err(|e| AppError::parse(format!("invalid benchmark JSON: {e}")))
}

/// Parses the benchmark artifact.
///
/// The file is named `yaml` but is JSON. This is not a mistake to be "fixed":
/// see the regression test at the bottom of this module.
pub fn parse_benchmark(text: &str) -> AppResult<BenchmarkMetrics> {
    let value = parse_benchmark_json(text)?;

    let object = value
        .as_object()
        .ok_or_else(|| AppError::parse("benchmark artifact is not a JSON object"))?;

    let metric = |canonical: &str| -> AppResult<f64> {
        let aliases = METRIC_ALIASES
            .iter()
            .find(|(key, _)| *key == canonical)
            .map(|(_, aliases)| *aliases)
            .unwrap_or(&[]);

        for alias in aliases {
            if let Some(raw) = object.get(*alias).and_then(serde_json::Value::as_f64) {
                return Ok(round2(raw));
            }
        }
        Err(AppError::parse(format!(
            "missing or non-numeric {}",
            aliases.join(" / ")
        )))
    };

    let median_ttft_ms = metric("median_ttft_ms")?;
    let median_tpot_ms = metric("median_tpot_ms")?;

    let (median_e2el_ms, e2el_approximate) = match metric("median_e2el_ms") {
        Ok(value) => (value, false),
        Err(missing) => (
            approximate_e2el(object, median_ttft_ms, median_tpot_ms).ok_or(missing)?,
            true,
        ),
    };

    Ok(BenchmarkMetrics {
        median_itl_ms: metric("median_itl_ms")?,
        median_ttft_ms,
        median_tpot_ms,
        median_e2el_ms,
        e2el_approximate,
        output_throughput: metric("output_throughput")?,
        total_token_throughput: metric("total_token_throughput")?,
    })
}

/// Rebuilds end-to-end latency from the per-token numbers.
///
/// `vllm bench serve` only writes the latency blocks named in
/// `--percentile-metrics`, and its default for generative models is
/// `ttft,tpot,itl`, so a sweep launched without `e2el` produces an artifact with
/// no end-to-end field at all. A request's end-to-end time is its first token
/// plus one TPOT per remaining token, and the output length per request comes
/// from the same artifact, so the run is worth approximating rather than
/// rejecting. Flagged as approximate, since the median of sums is not the sum of
/// medians.
fn approximate_e2el(
    object: &serde_json::Map<String, serde_json::Value>,
    ttft_ms: f64,
    tpot_ms: f64,
) -> Option<f64> {
    let completed = object.get("completed")?.as_f64()?;
    let output_tokens = object.get("total_output_tokens")?.as_f64()?;
    if completed <= 0.0 || output_tokens <= 0.0 {
        return None;
    }

    let tokens_per_request = output_tokens / completed;
    Some(round2(
        ttft_ms + tpot_ms * (tokens_per_request - 1.0).max(0.0),
    ))
}

/// Reads `max_concurrency` from the benchmark artifact, which is the value the
/// CLI treats as authoritative for the row.
pub fn parse_benchmark_concurrency(text: &str) -> AppResult<i64> {
    parse_benchmark_json(text)?
        .get("max_concurrency")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| AppError::parse("missing or non-integer max_concurrency"))
}

/// Input/output lengths as recorded in the benchmark artifact, when present.
///
/// The sglang harness writes these alongside the metrics; the vllm one does not.
/// Used only as a fallback when `commands.txt` does not carry them, so the primary
/// source stays the same as the CLI's.
pub fn parse_benchmark_dims(text: &str) -> Option<(i64, i64)> {
    let value = parse_benchmark_json(text).ok()?;
    let input = value.get("random_input_len")?.as_i64()?;
    let output = value.get("random_output_len")?.as_i64()?;
    Some((input, output))
}

/// Rounds to two decimals the way Python's `round()` does, because the CLI rounds
/// every metric before writing and our numbers have to match it.
///
/// Formatting performs a correctly-rounded decimal conversion of the true binary
/// value, using the same round-half-to-even tie rule as Python. The obvious
/// `(v * 100.0).round() / 100.0` is wrong twice over: it rounds halves away from
/// zero, and the multiply itself double-rounds. `2.675` is stored as
/// `2.67499999…`, yet `2.675 * 100.0` lands on exactly `267.5`, so scaling first
/// yields `2.68` where Python gives `2.67`.
pub fn round2(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    format!("{value:.2}").parse().unwrap_or(value)
}

pub fn group_key(input_len: i64, output_len: i64) -> String {
    format!("in{input_len}_out{output_len}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_COMMANDS: &str = r#"# ============================================================
# COMMAND SNIPPETS - Generated by inference-testing
# ============================================================
# Hostname: ip-172-31-48-204
# Image: rocm/vllm-private:rocm7.2.1_kineto

docker run \
  --rm \
  --name vllm-private \
  -e HF_TOKEN="hf_secret" \
  -e VLLM_ROCM_USE_AITER="1" \
  -e VLLM_ROCM_USE_AITER_MHA="0" \
  -e HIP_VISIBLE_DEVICES="6" \
  --entrypoint "" \
  --network host \
  --shm-size 64G \
  vllm serve amd/gpt-oss-120b-w-mxfp4-a-fp8 \
    --host 0.0.0.0 \
    --port 8022 \
    --async_scheduling \
    --tensor_parallel_size 1 \
    --kv-cache-dtype fp8 \
    --profiler-config.torch_profiler_dir /tmp/traces1

# Command 1: vllm_bench_serve
vllm bench serve \
  --backend vllm \
  --port 8022 \
  --model amd/gpt-oss-120b-w-mxfp4-a-fp8 \
  --random_input_len 1000 \
  --random_output_len 100 \
  --max_concurrency 16 \
  --num_prompts 16
"#;

    #[test]
    fn parses_env_vars_with_quotes_from_continuation_lines() {
        let meta = parse_commands(REAL_COMMANDS);
        assert_eq!(
            meta.env_vars.get("VLLM_ROCM_USE_AITER").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            meta.env_vars
                .get("VLLM_ROCM_USE_AITER_MHA")
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            meta.env_vars.get("HIP_VISIBLE_DEVICES").map(String::as_str),
            Some("6")
        );
    }

    #[test]
    fn parses_benchmark_flags() {
        let meta = parse_commands(REAL_COMMANDS);
        assert_eq!(meta.flag_i64("random_input_len").unwrap(), 1000);
        assert_eq!(meta.flag_i64("random_output_len").unwrap(), 100);
        assert_eq!(meta.flag_i64("max_concurrency").unwrap(), 16);
        assert_eq!(meta.flag("tensor_parallel_size").unwrap(), "1");
    }

    #[test]
    fn normalizes_dashed_flag_names_to_underscores() {
        let meta = parse_commands(REAL_COMMANDS);
        // Written as --kv-cache-dtype, addressable either way.
        assert_eq!(meta.flag("kv_cache_dtype").unwrap(), "fp8");
        assert_eq!(meta.flag("kv-cache-dtype").unwrap(), "fp8");
    }

    #[test]
    fn records_bare_switches_as_true() {
        let meta = parse_commands(REAL_COMMANDS);
        assert_eq!(meta.flag("async_scheduling").unwrap(), "true");
        assert_eq!(meta.flag("rm").unwrap(), "true");
    }

    #[test]
    fn a_flag_repeated_with_the_same_value_is_not_ambiguous() {
        // --port and --model appear in both the serve and bench commands.
        let meta = parse_commands(REAL_COMMANDS);
        assert_eq!(meta.flag("port").unwrap(), "8022");
        assert_eq!(
            meta.flag("model").unwrap(),
            "amd/gpt-oss-120b-w-mxfp4-a-fp8"
        );
    }

    #[test]
    fn a_flag_repeated_with_conflicting_values_is_ambiguous() {
        let meta = parse_commands("--port 8022\n--port 9000\n");
        let err = meta.flag("port").unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
    }

    #[test]
    fn skips_dotted_flags_like_the_cli_does() {
        // `--profiler-config.torch_profiler_dir` contains a dot, which the CLI's
        // flag pattern rejects. Matching that keeps our flag set identical.
        let meta = parse_commands(REAL_COMMANDS);
        assert!(!meta.flags.keys().any(|k| k.contains("profiler_config")));
    }

    #[test]
    fn missing_flag_reports_the_dashed_name() {
        let meta = parse_commands("--port 1\n");
        let err = meta.flag("random_input_len").unwrap_err().to_string();
        assert!(err.contains("--random-input-len"), "{err}");
    }

    #[test]
    fn compare_values_fall_back_to_image_default() {
        let meta = parse_commands(REAL_COMMANDS);
        assert_eq!(meta.compare_value("tensor_parallel_size").unwrap(), "1");
        assert_eq!(meta.compare_value("env:VLLM_ROCM_USE_AITER").unwrap(), "1");
        assert_eq!(
            meta.compare_value("env:NOT_SET_ANYWHERE").unwrap(),
            IMAGE_DEFAULT
        );
        assert_eq!(meta.compare_value("no_such_flag").unwrap(), IMAGE_DEFAULT);
    }

    #[test]
    fn comparison_headers_drop_the_env_prefix() {
        assert_eq!(
            comparison_header("env:VLLM_ROCM_USE_AITER"),
            "VLLM_ROCM_USE_AITER"
        );
        assert_eq!(
            comparison_header("tensor_parallel_size"),
            "tensor_parallel_size"
        );
    }

    // The artifact is named `yaml` but contains JSON. Anyone "fixing" this to
    // use a YAML parser will break every load, so it is pinned by a test.
    #[test]
    fn benchmark_artifact_named_yaml_is_parsed_as_json() {
        let text = r#"{
            "max_concurrency": 16,
            "median_itl_ms": 29.49584199814126,
            "median_ttft_ms": 1225.3003310179338,
            "median_tpot_ms": 29.672150500118732,
            "median_e2el_ms": 4161.693745001685,
            "output_throughput": 384.02979945866105,
            "total_token_throughput": 4224.327794045272
        }"#;

        let metrics = parse_benchmark(text).unwrap();
        // Rounded to two decimals, exactly as the CLI writes them.
        assert_eq!(metrics.median_itl_ms, 29.50);
        assert_eq!(metrics.median_ttft_ms, 1225.30);
        assert_eq!(metrics.median_tpot_ms, 29.67);
        assert_eq!(metrics.median_e2el_ms, 4161.69);
        assert_eq!(metrics.output_throughput, 384.03);
        assert_eq!(metrics.total_token_throughput, 4224.33);
        assert_eq!(parse_benchmark_concurrency(text).unwrap(), 16);
    }

    // Trimmed from a real sglang.bench_serving artifact. Three things differ from
    // the vllm variant: the bare `Infinity` literal, `median_e2e_latency_ms` in
    // place of `median_e2el_ms`, and `total_throughput` in place of
    // `total_token_throughput`.
    const SGLANG_BENCHMARK: &str = r#"{
        "backend": "vllm",
        "dataset_name": "random",
        "request_rate": Infinity,
        "sharegpt_output_len": null,
        "max_concurrency": 256,
        "random_input_len": 128000,
        "random_output_len": 1024,
        "output_throughput": 392.68562839175024,
        "total_throughput": 49491.299923230494,
        "median_e2e_latency_ms": 594877.428857144,
        "median_ttft_ms": 505895.77354630455,
        "median_tpot_ms": 98.67288582891271,
        "median_itl_ms": 17.123665660619736,
        "accept_length": null
    }"#;

    #[test]
    fn parses_the_sglang_metric_spelling() {
        let metrics = parse_benchmark(SGLANG_BENCHMARK).unwrap();
        assert_eq!(metrics.median_itl_ms, 17.12);
        assert_eq!(metrics.median_ttft_ms, 505895.77);
        assert_eq!(metrics.median_tpot_ms, 98.67);
        // Read from median_e2e_latency_ms.
        assert_eq!(metrics.median_e2el_ms, 594877.43);
        assert_eq!(metrics.output_throughput, 392.69);
        // Read from total_throughput.
        assert_eq!(metrics.total_token_throughput, 49491.3);

        assert_eq!(parse_benchmark_concurrency(SGLANG_BENCHMARK).unwrap(), 256);
    }

    // Bare `Infinity` is invalid JSON, and serde_json rejects the whole document
    // over it, so the sanitizer is load-bearing for every sglang run.
    #[test]
    fn tolerates_bare_infinity_and_nan_literals() {
        assert!(serde_json::from_str::<serde_json::Value>(SGLANG_BENCHMARK).is_err());
        assert!(parse_benchmark(SGLANG_BENCHMARK).is_ok());

        for literal in ["Infinity", "-Infinity", "NaN", "-NaN"] {
            let text = format!(
                r#"{{"request_rate": {literal}, "max_concurrency": 4,
                    "median_itl_ms": 1.0, "median_ttft_ms": 2.0, "median_tpot_ms": 3.0,
                    "median_e2el_ms": 4.0, "output_throughput": 5.0,
                    "total_token_throughput": 6.0}}"#
            );
            assert!(parse_benchmark(&text).is_ok(), "failed on {literal}");
        }
    }

    #[test]
    fn sanitizing_leaves_the_same_word_inside_a_string_alone() {
        let text = r#"{"label": "Infinity pool", "note": "NaN"}"#;
        assert_eq!(sanitize_json_constants(text), text);
    }

    #[test]
    fn sanitizing_handles_adjacent_literals_in_an_array() {
        let out = sanitize_json_constants(r#"{"xs": [NaN, NaN, Infinity]}"#);
        assert!(!out.contains("NaN"), "{out}");
        assert!(!out.contains("Infinity"), "{out}");
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
    }

    #[test]
    fn sanitizing_is_a_no_op_for_ordinary_json() {
        let text = r#"{"a": 1.0}"#;
        assert!(matches!(
            sanitize_json_constants(text),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn reads_dimensions_from_the_artifact_when_present() {
        // sglang records these; vllm does not.
        assert_eq!(parse_benchmark_dims(SGLANG_BENCHMARK), Some((128000, 1024)));
        assert_eq!(parse_benchmark_dims(r#"{"max_concurrency": 4}"#), None);
    }

    // The sglang harness writes dashed flags; ours normalize to underscores, so
    // the same accessors work for both.
    #[test]
    fn parses_dashed_sglang_bench_flags() {
        let meta = parse_commands(
            "python -m sglang.bench_serving \\\n  --random-input-len 128000 \\\n  --random-output-len 1024 \\\n  --max-concurrency 256 \\\n  --request-rate inf\n",
        );
        assert_eq!(meta.flag_i64("random_input_len").unwrap(), 128000);
        assert_eq!(meta.flag_i64("random_output_len").unwrap(), 1024);
        assert_eq!(meta.flag_i64("max_concurrency").unwrap(), 256);
    }

    #[test]
    fn benchmark_parse_rejects_missing_metrics() {
        let err = parse_benchmark(r#"{"median_itl_ms": 1.0}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("median_ttft_ms"), "{err}");

        assert!(parse_benchmark("not json at all").is_err());
        assert!(parse_benchmark("[1, 2, 3]").is_err());
    }

    // A recent `vllm bench serve` run launched without `e2el` in
    // --percentile-metrics: complete ttft/tpot/itl blocks and no e2el field.
    #[test]
    fn approximates_e2el_when_the_harness_omitted_it() {
        let text = r#"{
            "max_concurrency": 8,
            "completed": 20,
            "total_output_tokens": 20480,
            "median_ttft_ms": 30799.74504650454,
            "median_tpot_ms": 206.77968056210526,
            "median_itl_ms": 43.19396149367094,
            "output_throughput": 32.48393290743179,
            "total_token_throughput": 4190.427345058702
        }"#;

        let metrics = parse_benchmark(text).unwrap();
        assert!(metrics.e2el_approximate);
        // 1024 output tokens per request: 30799.75 + 206.78 * 1023.
        assert_eq!(metrics.median_e2el_ms, 242335.69);

        // Without the token counts there is nothing to derive from, so the run is
        // still rejected rather than silently reported as zero.
        let bare = r#"{"median_ttft_ms": 1.0, "median_tpot_ms": 2.0, "median_itl_ms": 3.0,
            "output_throughput": 4.0, "total_token_throughput": 5.0}"#;
        let err = parse_benchmark(bare).unwrap_err().to_string();
        assert!(err.contains("median_e2el_ms"), "{err}");
    }

    #[test]
    fn a_reported_e2el_is_never_flagged_as_approximate() {
        assert!(!parse_benchmark(SGLANG_BENCHMARK).unwrap().e2el_approximate);
    }

    #[test]
    fn metric_lookup_covers_every_exported_key() {
        let metrics = BenchmarkMetrics {
            median_itl_ms: 1.0,
            median_ttft_ms: 2.0,
            median_tpot_ms: 3.0,
            median_e2el_ms: 4.0,
            e2el_approximate: false,
            output_throughput: 5.0,
            total_token_throughput: 6.0,
        };
        for key in METRIC_KEYS {
            assert!(metrics.get(key).is_some(), "no accessor for {key}");
        }
        assert!(metrics.get("nonexistent").is_none());
    }

    #[test]
    fn group_keys_match_the_cli_format() {
        assert_eq!(group_key(1000, 100), "in1000_out100");
    }

    #[test]
    fn rounds_to_two_places() {
        assert_eq!(round2(1.2345), 1.23);
        assert_eq!(round2(1.2355), 1.24);
        assert_eq!(round2(1.0049), 1.0);
        assert_eq!(round2(-1.2355), -1.24);
        assert_eq!(round2(0.0), 0.0);
    }

    // Parity with Python's `round(v, 2)`, verified against CPython. Every value
    // here is one where the naive `(v * 100.0).round() / 100.0` disagrees, so this
    // is the test that keeps our exports matching `adb-summarize`.
    #[test]
    fn rounding_matches_python_on_ties_and_near_ties() {
        // Exact binary halves: Python rounds to even.
        assert_eq!(round2(0.125), 0.12); // not 0.13
        assert_eq!(round2(-0.125), -0.12);
        assert_eq!(round2(0.375), 0.38); // 37 is odd, so this one goes up

        // Not actually halves, though they look like it. Scaling by 100 turns
        // both into exact halves and rounds them the wrong way.
        assert_eq!(round2(2.675), 2.67); // stored as 2.67499999999999982
        assert_eq!(round2(-2.675), -2.67);
        assert_eq!(round2(0.135), 0.14); // stored as 0.13500000000000001
        assert_eq!(round2(1.005), 1.0); // stored as 1.00499999999999989
        assert_eq!(round2(-1.005), -1.0);
    }

    #[test]
    fn round2_passes_non_finite_values_through() {
        assert!(round2(f64::NAN).is_nan());
        assert_eq!(round2(f64::INFINITY), f64::INFINITY);
    }
}
