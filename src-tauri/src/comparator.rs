//! Builds the comparison table.
//!
//! Two sides are outer-joined on `(group, concurrency)` so a case present on one
//! side and missing on the other still produces a row, flagged rather than
//! dropped. Row order matches the CLI and the example CSV:
//! `(input_len, output_len, concurrency)`.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::model::{ChildRun, SideData};
use crate::parser::{self, BenchmarkMetrics, IMAGE_DEFAULT, METRIC_KEYS};

/// Metrics where a smaller number is better. Latency is better low, throughput
/// better high, and the ratio colouring is meaningless without knowing which.
const LOWER_IS_BETTER: [&str; 4] = [
    "median_itl_ms",
    "median_ttft_ms",
    "median_tpot_ms",
    "median_e2el_ms",
];

pub fn lower_is_better(metric: &str) -> bool {
    LOWER_IS_BETTER.contains(&metric)
}

/// Aggregation applied when several children share a `(group, concurrency)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Aggregation {
    #[default]
    Median,
    Mean,
    Max,
    Min,
}

/// One side's contribution to a row.
#[derive(Debug, Clone, Serialize)]
pub struct CellGroup {
    pub metrics: BenchmarkMetrics,
    /// How many children were aggregated into this cell.
    pub run_count: usize,
    pub run_ids: Vec<String>,
    /// Comparison-field values, keyed by display header.
    pub fields: BTreeMap<String, String>,
    pub benchmark_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonRow {
    pub group: String,
    pub input_len: i64,
    pub output_len: i64,
    pub concurrency: i64,
    pub a: Option<CellGroup>,
    pub b: Option<CellGroup>,
    /// Ratio per metric, as a percentage of A over B, present only when both
    /// sides have data and B is non-zero.
    pub ratios: BTreeMap<String, f64>,
}

impl ComparisonRow {
    pub fn is_matched(&self) -> bool {
        self.a.is_some() && self.b.is_some()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonTable {
    pub rows: Vec<ComparisonRow>,
    pub label_a: String,
    pub label_b: Option<String>,
    /// Comparison-field headers, in the configured order.
    pub field_headers: Vec<String>,
    pub metric_keys: Vec<String>,
    pub matched_rows: usize,
    pub a_only_rows: usize,
    pub b_only_rows: usize,
}

/// Joins one or two sides into a table.
///
/// `excluded_a` and `excluded_b` are the run IDs the user unticked in the case
/// list. They are filtered here rather than on the frontend because a cell can
/// aggregate several children, so dropping one changes the aggregate rather than
/// just removing a row.
pub fn build_table(
    a: &SideData,
    b: Option<&SideData>,
    compare_fields: &[String],
    aggregation: Aggregation,
    excluded_a: &HashSet<String>,
    excluded_b: &HashSet<String>,
) -> ComparisonTable {
    let field_headers: Vec<String> = compare_fields
        .iter()
        .map(|f| parser::comparison_header(f).to_string())
        .collect();

    let a_cells = group_side(&a.children, compare_fields, aggregation, excluded_a);
    let b_cells = b
        .map(|side| group_side(&side.children, compare_fields, aggregation, excluded_b))
        .unwrap_or_default();

    // BTreeMap keys give a deterministic union; the explicit sort below then
    // imposes the CSV's ordering, which is numeric rather than lexicographic.
    let mut keys: Vec<RowKey> = a_cells.keys().chain(b_cells.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut rows = Vec::with_capacity(keys.len());
    let (mut matched, mut a_only, mut b_only) = (0usize, 0usize, 0usize);

    for key in keys {
        let cell_a = a_cells.get(&key).cloned();
        let cell_b = b_cells.get(&key).cloned();

        match (&cell_a, &cell_b) {
            (Some(_), Some(_)) => matched += 1,
            (Some(_), None) => a_only += 1,
            (None, Some(_)) => b_only += 1,
            (None, None) => {}
        }

        let ratios = match (&cell_a, &cell_b) {
            (Some(x), Some(y)) => ratio_map(&x.metrics, &y.metrics),
            _ => BTreeMap::new(),
        };

        rows.push(ComparisonRow {
            group: parser::group_key(key.input_len, key.output_len),
            input_len: key.input_len,
            output_len: key.output_len,
            concurrency: key.concurrency,
            a: cell_a,
            b: cell_b,
            ratios,
        });
    }

    ComparisonTable {
        rows,
        label_a: a.label.clone(),
        label_b: b.map(|side| side.label.clone()),
        field_headers,
        metric_keys: METRIC_KEYS.iter().map(|s| s.to_string()).collect(),
        matched_rows: matched,
        a_only_rows: a_only,
        b_only_rows: b_only,
    }
}

/// Join key. Ordering is derived, so `(input, output, concurrency)` sorts
/// numerically, which is what the example CSV does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RowKey {
    input_len: i64,
    output_len: i64,
    concurrency: i64,
}

fn group_side(
    children: &[ChildRun],
    compare_fields: &[String],
    aggregation: Aggregation,
    excluded: &HashSet<String>,
) -> BTreeMap<RowKey, CellGroup> {
    let mut buckets: BTreeMap<RowKey, Vec<&ChildRun>> = BTreeMap::new();

    for child in children {
        if excluded.contains(&child.run_id) {
            continue;
        }
        buckets
            .entry(RowKey {
                input_len: child.input_len,
                output_len: child.output_len,
                concurrency: child.concurrency,
            })
            .or_default()
            .push(child);
    }

    buckets
        .into_iter()
        .map(|(key, group)| {
            let metrics = aggregate(&group, aggregation);

            // Field values come from the first child in the bucket; they describe
            // the configuration, which is by definition shared across a bucket.
            let mut fields = BTreeMap::new();
            if let Some(first) = group.first() {
                for field in compare_fields {
                    let header = parser::comparison_header(field).to_string();
                    let value = first
                        .metadata
                        .compare_value(field)
                        .unwrap_or_else(|_| IMAGE_DEFAULT.to_string());
                    fields.insert(header, value);
                }
            }

            let cell = CellGroup {
                metrics,
                run_count: group.len(),
                run_ids: group.iter().map(|c| c.run_id.clone()).collect(),
                fields,
                benchmark_path: group
                    .first()
                    .map(|c| c.benchmark_path.clone())
                    .unwrap_or_default(),
            };
            (key, cell)
        })
        .collect()
}

fn aggregate(group: &[&ChildRun], aggregation: Aggregation) -> BenchmarkMetrics {
    let pick = |key: &str| -> f64 {
        let mut values: Vec<f64> = group
            .iter()
            .filter_map(|child| child.metrics.get(key))
            .collect();
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));

        let value = match aggregation {
            Aggregation::Mean => values.iter().sum::<f64>() / values.len() as f64,
            Aggregation::Max => *values.last().unwrap_or(&0.0),
            Aggregation::Min => *values.first().unwrap_or(&0.0),
            Aggregation::Median => median_of_sorted(&values),
        };
        parser::round2(value)
    };

    BenchmarkMetrics {
        median_itl_ms: pick("median_itl_ms"),
        median_ttft_ms: pick("median_ttft_ms"),
        median_tpot_ms: pick("median_tpot_ms"),
        median_e2el_ms: pick("median_e2el_ms"),
        // One estimated child makes the aggregate an estimate.
        e2el_approximate: group.iter().any(|child| child.metrics.e2el_approximate),
        output_throughput: pick("output_throughput"),
        total_token_throughput: pick("total_token_throughput"),
    }
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        // Even count: average the two middle values.
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// A over B as a percentage, per metric.
///
/// Skips a metric when B is zero rather than emitting an infinity, which would
/// serialize to `null` in JSON and render as a confusing blank.
fn ratio_map(a: &BenchmarkMetrics, b: &BenchmarkMetrics) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for key in METRIC_KEYS {
        if let (Some(x), Some(y)) = (a.get(key), b.get(key)) {
            if y != 0.0 && y.is_finite() && x.is_finite() {
                out.insert(key.to_string(), parser::round2(x / y * 100.0));
            }
        }
    }
    out
}

/// A point on a curve, used by the chart tab.
///
/// Carries the full `(input_len, output_len, concurrency)` identity, not just the
/// plotted x, so a tooltip can name the benchmark case regardless of which of the
/// three the x axis happens to be showing.
#[derive(Debug, Clone, Serialize)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
    pub group: String,
    pub input_len: i64,
    pub output_len: i64,
    pub concurrency: i64,
    /// How many child runs were aggregated into this point.
    pub run_count: usize,
    pub is_pareto: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurveSeries {
    pub label: String,
    pub points: Vec<CurvePoint>,
}

/// X axis choices for the curve tab.
///
/// Only knobs belong here. `input_len` and `output_len` define the workload
/// rather than trading off against anything, so they select the case instead of
/// forming an axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum XAxis {
    #[default]
    Concurrency,
    /// Per-user output speed, `1000 / median_tpot_ms` tokens per second.
    Interactivity,
}

impl XAxis {
    /// Whether a smaller x is the better outcome, which the frontier sweep needs.
    /// Concurrency is a cost knob, interactivity is a result users feel.
    fn lower_is_better(self) -> bool {
        matches!(self, XAxis::Concurrency)
    }
}

/// Builds one series per side, with the Pareto frontier flagged.
///
/// `case` restricts the series to a single `(input_len, output_len)` workload.
/// Without it, points from unrelated workloads would be joined into one line.
pub fn build_series(
    side: &SideData,
    metric: &str,
    x_axis: XAxis,
    aggregation: Aggregation,
    case: Option<(i64, i64)>,
    excluded: &HashSet<String>,
) -> CurveSeries {
    let cells = group_side(&side.children, &[], aggregation, excluded);

    let mut points: Vec<CurvePoint> = cells
        .into_iter()
        .filter(|(key, _)| match case {
            Some((isl, osl)) => key.input_len == isl && key.output_len == osl,
            None => true,
        })
        .filter_map(|(key, cell)| {
            let y = cell.metrics.get(metric)?;
            let x = match x_axis {
                XAxis::Concurrency => key.concurrency as f64,
                XAxis::Interactivity => {
                    let tpot = cell.metrics.median_tpot_ms;
                    if tpot <= 0.0 || !tpot.is_finite() {
                        return None;
                    }
                    parser::round2(1000.0 / tpot)
                }
            };
            Some(CurvePoint {
                x,
                y,
                group: parser::group_key(key.input_len, key.output_len),
                input_len: key.input_len,
                output_len: key.output_len,
                concurrency: key.concurrency,
                run_count: cell.run_count,
                is_pareto: false,
            })
        })
        .collect();

    points.sort_by(|p, q| p.x.partial_cmp(&q.x).unwrap_or(std::cmp::Ordering::Equal));
    mark_pareto(
        &mut points,
        lower_is_better(metric),
        x_axis.lower_is_better(),
    );

    CurveSeries {
        label: side.label.clone(),
        points,
    }
}

/// Flags non-dominated points.
///
/// A point is dominated when another one is at least as good on x and strictly
/// better on y. Sweeping from the best-x end and tracking the best y so far is
/// enough; which end that is depends on whether small or large x is preferable.
fn mark_pareto(points: &mut [CurvePoint], y_lower_better: bool, x_lower_better: bool) {
    let mut best: Option<f64> = None;

    // Input is sorted by ascending x, so a preference for large x walks it backwards.
    let ordered: Vec<&mut CurvePoint> = if x_lower_better {
        points.iter_mut().collect()
    } else {
        points.iter_mut().rev().collect()
    };

    for point in ordered {
        let improves = match best {
            None => true,
            Some(current) => {
                if y_lower_better {
                    point.y < current
                } else {
                    point.y > current
                }
            }
        };

        if improves {
            point.is_pareto = true;
            best = Some(point.y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChildRun, SideSource};
    use crate::parser::parse_commands;

    fn child(input: i64, output: i64, concurrency: i64, throughput: f64, tpot: f64) -> ChildRun {
        ChildRun {
            run_id: format!("r{input}_{output}_{concurrency}_{throughput}"),
            run_name: format!("c{concurrency}"),
            status: Some("FINISHED".into()),
            start_time: None,
            group: parser::group_key(input, output),
            input_len: input,
            output_len: output,
            concurrency,
            metrics: BenchmarkMetrics {
                median_itl_ms: 10.0,
                median_ttft_ms: 20.0,
                median_tpot_ms: tpot,
                median_e2el_ms: 40.0,
                e2el_approximate: false,
                output_throughput: throughput,
                total_token_throughput: throughput * 10.0,
            },
            metadata: parse_commands("--tensor_parallel_size 4\n-e VLLM_ROCM_USE_AITER=1\n"),
            benchmark_path: "benchmark_results/0_vllm_bench_serve/yaml".into(),
        }
    }

    fn side(label: &str, children: Vec<ChildRun>) -> SideData {
        SideData {
            run_id: "run".into(),
            run_name: label.into(),
            experiment_id: "1".into(),
            experiment_name: None,
            is_parent: true,
            label: label.into(),
            children,
            failures: Vec::new(),
            from_cache: false,
            source: SideSource::Remote,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn joins_matching_rows_and_computes_ratios() {
        let a = side("A", vec![child(1000, 100, 4, 200.0, 30.0)]);
        let b = side("B", vec![child(1000, 100, 4, 100.0, 60.0)]);

        let table = build_table(
            &a,
            Some(&b),
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.matched_rows, 1);
        let row = &table.rows[0];
        assert!(row.is_matched());
        // A has double the throughput and half the TPOT.
        assert_eq!(row.ratios.get("output_throughput").copied(), Some(200.0));
        assert_eq!(row.ratios.get("median_tpot_ms").copied(), Some(50.0));
    }

    #[test]
    fn outer_join_keeps_rows_present_on_only_one_side() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 200.0, 30.0),
                child(1000, 100, 8, 300.0, 35.0),
            ],
        );
        let b = side(
            "B",
            vec![
                child(1000, 100, 4, 100.0, 60.0),
                child(1000, 100, 16, 400.0, 40.0),
            ],
        );

        let table = build_table(
            &a,
            Some(&b),
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.matched_rows, 1);
        assert_eq!(table.a_only_rows, 1);
        assert_eq!(table.b_only_rows, 1);

        let a_only = table.rows.iter().find(|r| r.concurrency == 8).unwrap();
        assert!(a_only.a.is_some() && a_only.b.is_none());
        assert!(a_only.ratios.is_empty(), "no ratios without both sides");
    }

    #[test]
    fn rows_sort_numerically_not_lexicographically() {
        // Lexicographic ordering would put concurrency 16 before 4.
        let a = side(
            "A",
            vec![
                child(1000, 100, 16, 1.0, 1.0),
                child(1000, 100, 4, 1.0, 1.0),
                child(500, 100, 8, 1.0, 1.0),
                child(1000, 50, 32, 1.0, 1.0),
            ],
        );

        let table = build_table(
            &a,
            None,
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );
        let order: Vec<(i64, i64, i64)> = table
            .rows
            .iter()
            .map(|r| (r.input_len, r.output_len, r.concurrency))
            .collect();

        assert_eq!(
            order,
            vec![
                (500, 100, 8),
                (1000, 50, 32),
                (1000, 100, 4),
                (1000, 100, 16)
            ]
        );
    }

    #[test]
    fn single_side_table_has_no_b_column() {
        let a = side("A", vec![child(1000, 100, 4, 200.0, 30.0)]);
        let table = build_table(
            &a,
            None,
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(table.label_b.is_none());
        assert!(table.rows[0].b.is_none());
    }

    #[test]
    fn aggregates_duplicates_by_the_chosen_statistic() {
        let children = vec![
            child(1000, 100, 4, 100.0, 10.0),
            child(1000, 100, 4, 200.0, 20.0),
            child(1000, 100, 4, 300.0, 30.0),
        ];
        let a = side("A", children);

        let median = build_table(
            &a,
            None,
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );
        let cell = median.rows[0].a.as_ref().unwrap();
        assert_eq!(cell.metrics.output_throughput, 200.0);
        assert_eq!(cell.run_count, 3);

        let throughput = |aggregation| {
            build_table(&a, None, &[], aggregation, &HashSet::new(), &HashSet::new()).rows[0]
                .a
                .as_ref()
                .unwrap()
                .metrics
                .output_throughput
        };

        assert_eq!(throughput(Aggregation::Mean), 200.0);
        assert_eq!(throughput(Aggregation::Max), 300.0);
        assert_eq!(throughput(Aggregation::Min), 100.0);
    }

    // Excluding a child has to change the aggregate, not just drop a row, which
    // is why the filter lives here rather than in the table component.
    #[test]
    fn excluding_a_child_recomputes_the_aggregate() {
        let children = vec![
            child(1000, 100, 4, 100.0, 10.0),
            child(1000, 100, 4, 200.0, 20.0),
            child(1000, 100, 4, 300.0, 30.0),
        ];
        let slowest = children[0].run_id.clone();
        let a = side("A", children);

        let excluded: HashSet<String> = [slowest].into_iter().collect();
        let table = build_table(
            &a,
            None,
            &[],
            Aggregation::Median,
            &excluded,
            &HashSet::new(),
        );

        let cell = table.rows[0].a.as_ref().unwrap();
        // Median of the two survivors, not of all three.
        assert_eq!(cell.metrics.output_throughput, 250.0);
        assert_eq!(cell.run_count, 2);
    }

    #[test]
    fn excluding_every_child_of_a_side_leaves_the_other_side_intact() {
        let a = side("A", vec![child(1000, 100, 4, 200.0, 30.0)]);
        let b = side("B", vec![child(1000, 100, 4, 100.0, 60.0)]);
        let all_of_b: HashSet<String> = b.children.iter().map(|c| c.run_id.clone()).collect();

        let table = build_table(
            &a,
            Some(&b),
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &all_of_b,
        );

        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.matched_rows, 0);
        assert_eq!(table.a_only_rows, 1);
        assert!(table.rows[0].b.is_none());
        assert!(table.rows[0].ratios.is_empty());
    }

    #[test]
    fn excluded_children_are_dropped_from_the_curve() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 100.0, 10.0),
                child(1000, 100, 8, 180.0, 12.0),
                child(1000, 100, 16, 220.0, 20.0),
            ],
        );
        let excluded: HashSet<String> = [a.children[1].run_id.clone()].into_iter().collect();

        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Concurrency,
            Aggregation::Median,
            None,
            &excluded,
        );

        let xs: Vec<f64> = series.points.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![4.0, 16.0]);
    }

    #[test]
    fn median_of_an_even_count_averages_the_middle_two() {
        assert_eq!(median_of_sorted(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median_of_sorted(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median_of_sorted(&[]), 0.0);
    }

    #[test]
    fn ratios_skip_a_zero_denominator_instead_of_emitting_infinity() {
        let a = side("A", vec![child(1000, 100, 4, 200.0, 30.0)]);
        let b = side("B", vec![child(1000, 100, 4, 0.0, 60.0)]);

        let table = build_table(
            &a,
            Some(&b),
            &[],
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );
        let row = &table.rows[0];
        assert!(!row.ratios.contains_key("output_throughput"));
        // Other metrics with non-zero denominators still get a ratio.
        assert!(row.ratios.contains_key("median_tpot_ms"));
    }

    #[test]
    fn populates_comparison_fields_with_headers_stripped() {
        let a = side("A", vec![child(1000, 100, 4, 200.0, 30.0)]);
        let fields = vec![
            "tensor_parallel_size".to_string(),
            "env:VLLM_ROCM_USE_AITER".to_string(),
            "env:NOT_PRESENT".to_string(),
        ];

        let table = build_table(
            &a,
            None,
            &fields,
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            table.field_headers,
            vec!["tensor_parallel_size", "VLLM_ROCM_USE_AITER", "NOT_PRESENT"]
        );

        let cell_fields = &table.rows[0].a.as_ref().unwrap().fields;
        assert_eq!(
            cell_fields.get("tensor_parallel_size").map(String::as_str),
            Some("4")
        );
        assert_eq!(
            cell_fields.get("VLLM_ROCM_USE_AITER").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            cell_fields.get("NOT_PRESENT").map(String::as_str),
            Some(IMAGE_DEFAULT)
        );
    }

    #[test]
    fn knows_which_metrics_improve_downward() {
        assert!(lower_is_better("median_tpot_ms"));
        assert!(lower_is_better("median_ttft_ms"));
        assert!(!lower_is_better("output_throughput"));
        assert!(!lower_is_better("total_token_throughput"));
    }

    #[test]
    fn pareto_frontier_tracks_increasing_throughput() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 100.0, 10.0),
                child(1000, 100, 8, 180.0, 12.0),
                // A regression at higher concurrency is dominated.
                child(1000, 100, 16, 150.0, 20.0),
                child(1000, 100, 32, 220.0, 25.0),
            ],
        );

        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Concurrency,
            Aggregation::Median,
            None,
            &HashSet::new(),
        );
        let flags: Vec<bool> = series.points.iter().map(|p| p.is_pareto).collect();
        assert_eq!(flags, vec![true, true, false, true]);
    }

    #[test]
    fn pareto_frontier_inverts_for_latency_metrics() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 100.0, 30.0),
                child(1000, 100, 8, 100.0, 20.0),
                child(1000, 100, 16, 100.0, 25.0),
            ],
        );

        let series = build_series(
            &a,
            "median_tpot_ms",
            XAxis::Concurrency,
            Aggregation::Median,
            None,
            &HashSet::new(),
        );
        let flags: Vec<bool> = series.points.iter().map(|p| p.is_pareto).collect();
        // Lower TPOT is better, so only the descending points count.
        assert_eq!(flags, vec![true, true, false]);
    }

    #[test]
    fn interactivity_axis_is_reciprocal_tpot() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 100.0, 20.0),
                child(1000, 100, 8, 180.0, 50.0),
            ],
        );
        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Interactivity,
            Aggregation::Median,
            None,
            &HashSet::new(),
        );
        let xs: Vec<f64> = series.points.iter().map(|p| p.x).collect();
        // 50 ms per token is 20 tok/s per user, 20 ms is 50 tok/s.
        assert_eq!(xs, vec![20.0, 50.0]);
    }

    // On the interactivity axis both axes improve upwards, so the frontier is the
    // upper-right envelope rather than a running best from the left.
    #[test]
    fn interactivity_frontier_keeps_the_upper_right_envelope() {
        let a = side(
            "A",
            vec![
                // Slowest per user but highest throughput: still the best of its kind.
                child(1000, 100, 32, 300.0, 100.0),
                // Dominated: less throughput and slower per user than mc64 below.
                child(1000, 100, 16, 150.0, 50.0),
                child(1000, 100, 64, 200.0, 25.0),
                child(1000, 100, 8, 100.0, 10.0),
            ],
        );
        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Interactivity,
            Aggregation::Median,
            None,
            &HashSet::new(),
        );
        let flags: Vec<(f64, bool)> = series.points.iter().map(|p| (p.x, p.is_pareto)).collect();
        assert_eq!(
            flags,
            vec![(10.0, true), (20.0, false), (40.0, true), (100.0, true)]
        );
    }

    #[test]
    fn series_can_be_restricted_to_one_case() {
        let a = side(
            "A",
            vec![
                child(500, 100, 4, 1.0, 1.0),
                child(1000, 100, 4, 2.0, 1.0),
                child(1000, 100, 8, 3.0, 1.0),
            ],
        );
        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Concurrency,
            Aggregation::Median,
            Some((1000, 100)),
            &HashSet::new(),
        );
        let xs: Vec<f64> = series.points.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![4.0, 8.0]);
    }

    // The tooltip has to name the benchmark case, so every point carries all three
    // identifying values even when only one of them is on the x axis.
    #[test]
    fn curve_points_identify_their_case_on_every_axis() {
        let a = side("A", vec![child(1000, 100, 8, 300.0, 35.0)]);

        for axis in [XAxis::Concurrency, XAxis::Interactivity] {
            let series = build_series(
                &a,
                "output_throughput",
                axis,
                Aggregation::Median,
                None,
                &HashSet::new(),
            );
            let point = &series.points[0];
            assert_eq!(point.input_len, 1000);
            assert_eq!(point.output_len, 100);
            assert_eq!(point.concurrency, 8);
            assert_eq!(point.group, "in1000_out100");
            assert_eq!(point.run_count, 1);
        }
    }

    #[test]
    fn curve_point_run_count_reflects_aggregation() {
        let a = side(
            "A",
            vec![
                child(1000, 100, 4, 100.0, 10.0),
                child(1000, 100, 4, 300.0, 30.0),
            ],
        );
        let series = build_series(
            &a,
            "output_throughput",
            XAxis::Concurrency,
            Aggregation::Median,
            None,
            &HashSet::new(),
        );
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.points[0].run_count, 2);
    }
}
