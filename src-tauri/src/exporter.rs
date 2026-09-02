//! Export writers.
//!
//! Three outputs: the comparison table with the two-row grouped header the
//! example CSV uses, the same thing as XLSX with real merged cells, and a flat
//! per-child CSV that stays column-compatible with `adb-summarize`.

use std::collections::HashSet;
use std::path::Path;

use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, Workbook};
use serde::Deserialize;

use crate::comparator::{lower_is_better, CellGroup, ComparisonTable};
use crate::error::{AppError, AppResult};
use crate::model::SideData;
use crate::parser::METRIC_KEYS;

/// Key columns shared by both sides, in the example CSV's order.
const KEY_HEADERS: [&str; 3] = ["random_input_len", "random_output_len", "max_concurrency"];

/// Ratio columns exported unless overridden. Throughput and TPOT are the two the
/// example CSV shows.
const DEFAULT_RATIO_METRICS: [&str; 2] = ["output_throughput", "median_tpot_ms"];

#[derive(Debug, Clone, Deserialize)]
pub struct ExportOptions {
    /// Overrides for the auto-derived side labels.
    #[serde(default)]
    pub label_a: Option<String>,
    #[serde(default)]
    pub label_b: Option<String>,
    /// Metrics to emit ratio columns for.
    #[serde(default)]
    pub ratio_metrics: Vec<String>,
    /// Metric columns to emit per side. Empty means all six.
    #[serde(default)]
    pub metric_keys: Vec<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            label_a: None,
            label_b: None,
            ratio_metrics: DEFAULT_RATIO_METRICS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            metric_keys: Vec::new(),
        }
    }
}

impl ExportOptions {
    fn ratios(&self) -> Vec<String> {
        if self.ratio_metrics.is_empty() {
            DEFAULT_RATIO_METRICS
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.ratio_metrics.clone()
        }
    }

    /// The metric columns to emit, always in the canonical order regardless of
    /// the order the caller listed them in.
    fn metrics(&self) -> Vec<String> {
        if self.metric_keys.is_empty() {
            return METRIC_KEYS.iter().map(|m| m.to_string()).collect();
        }
        let wanted: HashSet<&str> = self.metric_keys.iter().map(String::as_str).collect();
        let selected: Vec<String> = METRIC_KEYS
            .iter()
            .filter(|key| wanted.contains(*key))
            .map(|m| m.to_string())
            .collect();

        // A selection that matches nothing known would silently produce a table
        // with no metric columns at all, which is never what was meant.
        if selected.is_empty() {
            METRIC_KEYS.iter().map(|m| m.to_string()).collect()
        } else {
            selected
        }
    }

    fn resolved_label_a(&self, table: &ComparisonTable) -> String {
        self.label_a
            .clone()
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| table.label_a.clone())
    }

    fn resolved_label_b(&self, table: &ComparisonTable) -> Option<String> {
        self.label_b
            .clone()
            .filter(|l| !l.trim().is_empty())
            .or_else(|| table.label_b.clone())
    }
}

/// Column layout, computed once and shared by the CSV and XLSX writers so the
/// two formats cannot drift apart.
struct Layout {
    field_headers: Vec<String>,
    metric_keys: Vec<String>,
    ratio_metrics: Vec<String>,
    /// Columns per side: comparison fields, then the selected metrics.
    side_span: usize,
    has_b: bool,
}

impl Layout {
    fn new(table: &ComparisonTable, options: &ExportOptions) -> Self {
        let field_headers = table.field_headers.clone();
        let metric_keys = options.metrics();
        let side_span = field_headers.len() + metric_keys.len();

        // Ratios for metrics that are not shown would be columns with no
        // context, so the ratio set is intersected with the visible metrics.
        let visible: HashSet<&str> = metric_keys.iter().map(String::as_str).collect();
        let ratio_metrics: Vec<String> = options
            .ratios()
            .into_iter()
            .filter(|m| visible.contains(m.as_str()))
            .collect();

        Self {
            field_headers,
            metric_keys,
            ratio_metrics,
            side_span,
            has_b: table.label_b.is_some(),
        }
    }

    fn total_columns(&self) -> usize {
        KEY_HEADERS.len()
            + self.side_span
            + if self.has_b {
                self.side_span + self.ratio_metrics.len()
            } else {
                0
            }
    }

    fn a_start(&self) -> usize {
        KEY_HEADERS.len()
    }

    fn b_start(&self) -> usize {
        self.a_start() + self.side_span
    }

    fn ratio_start(&self) -> usize {
        self.b_start() + self.side_span
    }

    /// Second header row: the per-side field and metric names, then ratio names.
    fn sub_headers(&self) -> Vec<String> {
        let mut row: Vec<String> = KEY_HEADERS.iter().map(|_| String::new()).collect();
        for _ in 0..(if self.has_b { 2 } else { 1 }) {
            row.extend(self.field_headers.iter().cloned());
            row.extend(self.metric_keys.iter().cloned());
        }
        if self.has_b {
            row.extend(self.ratio_metrics.iter().map(|m| format!("{m} %")));
        }
        row
    }
}

/// One side's metric values for a row, aligned to the metric columns.
///
/// An absent side yields `None` per column rather than a short vector, so a
/// missing side leaves blank cells instead of shifting every column after it.
fn metric_values(cell: Option<&CellGroup>, layout: &Layout) -> Vec<Option<f64>> {
    match cell {
        Some(cell) => layout
            .metric_keys
            .iter()
            .map(|key| cell.metrics.get(key))
            .collect(),
        None => vec![None; layout.metric_keys.len()],
    }
}

fn field_values(cell: Option<&CellGroup>, layout: &Layout) -> Vec<String> {
    layout
        .field_headers
        .iter()
        .map(|header| {
            cell.and_then(|c| c.fields.get(header).cloned())
                .unwrap_or_default()
        })
        .collect()
}

fn format_number(value: f64) -> String {
    // Two decimals matches what the CLI writes, and keeps Excel from showing
    // long floating point tails.
    format!("{value:.2}")
}

/// Writes the comparison CSV with the two-row grouped header.
pub fn write_comparison_csv(
    path: &Path,
    table: &ComparisonTable,
    options: &ExportOptions,
) -> AppResult<()> {
    let layout = Layout::new(table, options);
    let mut writer = csv::WriterBuilder::new()
        .from_path(path)
        .map_err(|e| AppError::io(format!("could not write {}: {e}", path.display())))?;

    // Row 1: group spans. Only the first cell of each span carries the label,
    // which is how the example CSV encodes a merged header in flat CSV.
    let mut top = vec![String::new(); layout.total_columns()];
    for (idx, key) in KEY_HEADERS.iter().enumerate() {
        top[idx] = (*key).to_string();
    }
    if let Some(slot) = top.get_mut(layout.a_start()) {
        *slot = options.resolved_label_a(table);
    }
    if layout.has_b {
        if let Some(label_b) = options.resolved_label_b(table) {
            if let Some(slot) = top.get_mut(layout.b_start()) {
                *slot = label_b;
            }
        }
        if let Some(slot) = top.get_mut(layout.ratio_start()) {
            *slot = "A vs B".to_string();
        }
    }
    writer
        .write_record(&top)
        .map_err(|e| AppError::io(e.to_string()))?;

    writer
        .write_record(layout.sub_headers())
        .map_err(|e| AppError::io(e.to_string()))?;

    for row in &table.rows {
        let mut record: Vec<String> = vec![
            row.input_len.to_string(),
            row.output_len.to_string(),
            row.concurrency.to_string(),
        ];

        for cell in [row.a.as_ref(), row.b.as_ref()]
            .into_iter()
            .take(if layout.has_b { 2 } else { 1 })
        {
            record.extend(field_values(cell, &layout));
            record.extend(
                metric_values(cell, &layout)
                    .into_iter()
                    .map(|v| v.map(format_number).unwrap_or_default()),
            );
        }

        if layout.has_b {
            for metric in &layout.ratio_metrics {
                record.push(
                    row.ratios
                        .get(metric)
                        .map(|v| format_number(*v))
                        .unwrap_or_default(),
                );
            }
        }

        writer
            .write_record(&record)
            .map_err(|e| AppError::io(e.to_string()))?;
    }

    writer
        .flush()
        .map_err(|e| AppError::io(format!("could not flush {}: {e}", path.display())))?;
    Ok(())
}

/// Writes the comparison as XLSX with real merged header cells.
pub fn write_comparison_xlsx(
    path: &Path,
    table: &ComparisonTable,
    options: &ExportOptions,
) -> AppResult<()> {
    let layout = Layout::new(table, options);

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet
        .set_name("Comparison")
        .map_err(|e| AppError::io(e.to_string()))?;

    let group_header = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_background_color(Color::RGB(0x1F_2937))
        .set_font_color(Color::White)
        .set_border(FormatBorder::Thin);

    let sub_header = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_background_color(Color::RGB(0xF3_F4_F6))
        .set_border(FormatBorder::Thin)
        .set_text_wrap();

    let key_cell = Format::new().set_align(FormatAlign::Center);
    let number = Format::new().set_num_format("0.00");
    let better = Format::new()
        .set_num_format("0.0")
        .set_background_color(Color::RGB(0xDC_FC_E7));
    let worse = Format::new()
        .set_num_format("0.0")
        .set_background_color(Color::RGB(0xFE_E2_E2));
    let missing = Format::new()
        .set_align(FormatAlign::Center)
        .set_font_color(Color::RGB(0x9C_A3_AF));

    // Header row 1: merge each key column vertically, and each side block
    // horizontally. `merge_range` rejects a single cell, so the vertical merges
    // are what give the key columns a two-row-tall header.
    for (idx, key) in KEY_HEADERS.iter().enumerate() {
        sheet
            .merge_range(0, idx as u16, 1, idx as u16, key, &group_header)
            .map_err(|e| AppError::io(e.to_string()))?;
    }

    let merge_span = |sheet: &mut rust_xlsxwriter::Worksheet,
                      start: usize,
                      span: usize,
                      text: &str|
     -> AppResult<()> {
        if span == 0 {
            return Ok(());
        }
        let first = start as u16;
        let last = (start + span - 1) as u16;
        if first == last {
            sheet
                .write_string_with_format(0, first, text, &group_header)
                .map_err(|e| AppError::io(e.to_string()))?;
        } else {
            sheet
                .merge_range(0, first, 0, last, text, &group_header)
                .map_err(|e| AppError::io(e.to_string()))?;
        }
        Ok(())
    };

    merge_span(
        sheet,
        layout.a_start(),
        layout.side_span,
        &options.resolved_label_a(table),
    )?;

    if layout.has_b {
        let label_b = options.resolved_label_b(table).unwrap_or_default();
        merge_span(sheet, layout.b_start(), layout.side_span, &label_b)?;
        merge_span(
            sheet,
            layout.ratio_start(),
            layout.ratio_metrics.len(),
            "A vs B",
        )?;
    }

    // Header row 2.
    for (idx, header) in layout.sub_headers().iter().enumerate() {
        if idx < KEY_HEADERS.len() {
            continue; // already covered by the vertical merge
        }
        sheet
            .write_string_with_format(1, idx as u16, header, &sub_header)
            .map_err(|e| AppError::io(e.to_string()))?;
    }

    // Data rows.
    for (row_idx, row) in table.rows.iter().enumerate() {
        let excel_row = (row_idx + 2) as u32;

        for (idx, value) in [row.input_len, row.output_len, row.concurrency]
            .iter()
            .enumerate()
        {
            sheet
                .write_number_with_format(excel_row, idx as u16, *value as f64, &key_cell)
                .map_err(|e| AppError::io(e.to_string()))?;
        }

        let sides: Vec<(usize, Option<&CellGroup>)> = if layout.has_b {
            vec![
                (layout.a_start(), row.a.as_ref()),
                (layout.b_start(), row.b.as_ref()),
            ]
        } else {
            vec![(layout.a_start(), row.a.as_ref())]
        };

        for (start, cell) in sides {
            let mut col = start;

            for value in field_values(cell, &layout) {
                sheet
                    .write_string_with_format(excel_row, col as u16, &value, &key_cell)
                    .map_err(|e| AppError::io(e.to_string()))?;
                col += 1;
            }

            for value in metric_values(cell, &layout) {
                match value {
                    Some(v) => sheet
                        .write_number_with_format(excel_row, col as u16, v, &number)
                        .map_err(|e| AppError::io(e.to_string()))?,
                    None => sheet
                        .write_string_with_format(excel_row, col as u16, "—", &missing)
                        .map_err(|e| AppError::io(e.to_string()))?,
                };
                col += 1;
            }
        }

        if layout.has_b {
            for (idx, metric) in layout.ratio_metrics.iter().enumerate() {
                let col = (layout.ratio_start() + idx) as u16;
                match row.ratios.get(metric) {
                    Some(value) => {
                        // A ratio above 100% means A is larger. Whether that is
                        // good depends on the metric's direction.
                        let a_is_better = if lower_is_better(metric) {
                            *value < 100.0
                        } else {
                            *value > 100.0
                        };
                        let format = if a_is_better { &better } else { &worse };
                        sheet
                            .write_number_with_format(excel_row, col, *value, format)
                            .map_err(|e| AppError::io(e.to_string()))?;
                    }
                    None => {
                        sheet
                            .write_string_with_format(excel_row, col, "—", &missing)
                            .map_err(|e| AppError::io(e.to_string()))?;
                    }
                }
            }
        }
    }

    sheet
        .set_freeze_panes(2, 3)
        .map_err(|e| AppError::io(e.to_string()))?;
    for col in 0..layout.total_columns() {
        let width = if col < KEY_HEADERS.len() { 16.0 } else { 14.0 };
        sheet
            .set_column_width(col as u16, width)
            .map_err(|e| AppError::io(e.to_string()))?;
    }

    workbook
        .save(path)
        .map_err(|e| AppError::io(format!("could not write {}: {e}", path.display())))?;
    Ok(())
}

/// Renders the comparison as a GitHub-flavoured markdown table.
///
/// Markdown has no merged or two-row headers, so the grouped header the CSV and
/// XLSX use is flattened: side labels move to a line above the table and each
/// column is prefixed with its side, which keeps the header cells narrow enough
/// to read in a PR body.
pub fn comparison_markdown(table: &ComparisonTable, options: &ExportOptions) -> String {
    let layout = Layout::new(table, options);
    let label_a = options.resolved_label_a(table);
    let label_b = options.resolved_label_b(table);

    let mut out = String::new();
    out.push_str(&format!("**A** — {}\n", md_escape(&label_a)));
    if layout.has_b {
        if let Some(label) = &label_b {
            out.push_str(&format!("**B** — {}\n", md_escape(label)));
        }
    }
    out.push('\n');

    // Alignment is built alongside the headers so the two cannot fall out of
    // step: numbers read better right-aligned, but the comparison fields hold
    // free text and look wrong pushed to the right.
    const LEFT: &str = ":---";
    const RIGHT: &str = "---:";

    let mut headers: Vec<String> = vec!["ISL".into(), "OSL".into(), "conc".into()];
    let mut alignments: Vec<String> = vec![RIGHT.into(); headers.len()];

    for side in side_tags(&layout) {
        headers.extend(
            layout
                .field_headers
                .iter()
                .map(|h| format!("{side} {}", md_escape(h))),
        );
        alignments.extend(layout.field_headers.iter().map(|_| LEFT.to_string()));

        headers.extend(
            layout
                .metric_keys
                .iter()
                .map(|k| format!("{side} {}", metric_label(k))),
        );
        alignments.extend(layout.metric_keys.iter().map(|_| RIGHT.to_string()));
    }

    if layout.has_b {
        headers.extend(
            layout
                .ratio_metrics
                .iter()
                .map(|k| format!("A/B {}", metric_label(k))),
        );
        alignments.extend(layout.ratio_metrics.iter().map(|_| RIGHT.to_string()));
    }

    out.push_str(&md_row(&headers));
    out.push_str(&md_row(&alignments));

    for row in &table.rows {
        let mut cells: Vec<String> = vec![
            row.input_len.to_string(),
            row.output_len.to_string(),
            row.concurrency.to_string(),
        ];

        for cell in [row.a.as_ref(), row.b.as_ref()]
            .into_iter()
            .take(if layout.has_b { 2 } else { 1 })
        {
            cells.extend(field_values(cell, &layout).into_iter().map(|value| {
                if value.is_empty() {
                    "—".to_string()
                } else {
                    md_escape(&value)
                }
            }));
            cells.extend(
                metric_values(cell, &layout)
                    .into_iter()
                    .map(|v| v.map(format_number).unwrap_or_else(|| "—".to_string())),
            );
        }

        if layout.has_b {
            for metric in &layout.ratio_metrics {
                cells.push(match row.ratios.get(metric) {
                    Some(value) => format!("{value:.1}%"),
                    None => "—".to_string(),
                });
            }
        }

        out.push_str(&md_row(&cells));
    }

    out
}

/// Column prefixes, so a header reads `A TTFT (ms)` rather than repeating twice.
fn side_tags(layout: &Layout) -> Vec<&'static str> {
    if layout.has_b {
        vec!["A", "B"]
    } else {
        vec!["A"]
    }
}

fn md_row(cells: &[String]) -> String {
    format!("| {} |\n", cells.join(" | "))
}

/// A pipe inside a cell would end the column early, and a backslash would eat
/// the escape, so both are escaped. Newlines become spaces for the same reason.
fn md_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

/// Display names matching the ones the UI shows, so a pasted table reads the
/// same as the screen it was copied from.
fn metric_label(key: &str) -> &str {
    match key {
        "median_itl_ms" => "ITL (ms)",
        "median_ttft_ms" => "TTFT (ms)",
        "median_tpot_ms" => "TPOT (ms)",
        "median_e2el_ms" => "E2EL (ms)",
        "output_throughput" => "Output tok/s",
        "total_token_throughput" => "Total tok/s",
        other => other,
    }
}

/// Writes the flat per-child CSV.
///
/// Column set matches `adb-summarize`'s summary output plus the side and run
/// identity, and carries `benchmark_yaml_path` last for traceability.
pub fn write_raw_csv(path: &Path, sides: &[(&str, &SideData, &HashSet<String>)]) -> AppResult<()> {
    let mut writer = csv::WriterBuilder::new()
        .from_path(path)
        .map_err(|e| AppError::io(format!("could not write {}: {e}", path.display())))?;

    let mut headers: Vec<String> = vec![
        "side".into(),
        "label".into(),
        "run_id".into(),
        "run_name".into(),
        "group".into(),
        "random_input_len".into(),
        "random_output_len".into(),
        "max_concurrency".into(),
    ];
    headers.extend(METRIC_KEYS.iter().map(|m| m.to_string()));
    headers.push("benchmark_yaml_path".into());

    writer
        .write_record(&headers)
        .map_err(|e| AppError::io(e.to_string()))?;

    for (side_name, side, excluded) in sides {
        for child in side
            .children
            .iter()
            .filter(|child| !excluded.contains(&child.run_id))
        {
            let mut record: Vec<String> = vec![
                (*side_name).to_string(),
                side.label.clone(),
                child.run_id.clone(),
                child.run_name.clone(),
                child.group.clone(),
                child.input_len.to_string(),
                child.output_len.to_string(),
                child.concurrency.to_string(),
            ];
            record.extend(METRIC_KEYS.iter().map(|key| {
                child
                    .metrics
                    .get(key)
                    .map(format_number)
                    .unwrap_or_default()
            }));
            record.push(child.benchmark_path.clone());

            writer
                .write_record(&record)
                .map_err(|e| AppError::io(e.to_string()))?;
        }
    }

    writer
        .flush()
        .map_err(|e| AppError::io(format!("could not flush {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparator::{build_table, Aggregation};
    use crate::model::{ChildRun, SideSource};
    use crate::parser::{parse_commands, BenchmarkMetrics};

    fn child(concurrency: i64, throughput: f64) -> ChildRun {
        ChildRun {
            run_id: format!("run{concurrency}"),
            run_name: format!("child{concurrency}"),
            status: Some("FINISHED".into()),
            start_time: None,
            group: "in1000_out100".into(),
            input_len: 1000,
            output_len: 100,
            concurrency,
            metrics: BenchmarkMetrics {
                median_itl_ms: 29.5,
                median_ttft_ms: 1225.3,
                median_tpot_ms: 29.67,
                median_e2el_ms: 4161.69,
                e2el_approximate: false,
                output_throughput: throughput,
                total_token_throughput: throughput * 11.0,
            },
            metadata: parse_commands("--tensor_parallel_size 4\n-e VLLM_ROCM_USE_AITER=1\n"),
            benchmark_path: "benchmark_results/0_vllm_bench_serve/yaml".into(),
        }
    }

    fn side(label: &str, children: Vec<ChildRun>) -> SideData {
        SideData {
            run_id: "r".into(),
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

    /// Table builder for the tests, which never exercise the exclusion filter;
    /// that is covered in `comparator`.
    fn table_of(a: &SideData, b: Option<&SideData>, fields: &[String]) -> ComparisonTable {
        build_table(
            a,
            b,
            fields,
            Aggregation::Median,
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "adb-vizzard-export-{}-{name}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn comparison_csv_has_a_two_row_grouped_header() {
        let a = side("MI355X", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B200", vec![child(4, 100.0), child(8, 150.0)]);
        let fields = vec!["tensor_parallel_size".to_string()];
        let table = table_of(&a, Some(&b), &fields);

        let path = temp_path("compare.csv");
        write_comparison_csv(&path, &table, &ExportOptions::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        // Row 1 carries the key names and one label per side block.
        assert!(lines[0].starts_with("random_input_len,random_output_len,max_concurrency,MI355X"));
        assert!(lines[0].contains("B200"));
        assert!(lines[0].contains("A vs B"));

        // Row 2 blanks the key columns and names every per-side column.
        assert!(lines[1].starts_with(",,,"));
        assert!(lines[1].contains("tensor_parallel_size"));
        assert!(lines[1].contains("output_throughput"));

        // One data row per matched case.
        assert_eq!(lines.len(), 4);
        assert!(lines[2].starts_with("1000,100,4,"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn single_side_csv_omits_the_b_block_and_ratios() {
        let a = side("Solo", vec![child(4, 200.0)]);
        let table = table_of(&a, None, &[]);

        let path = temp_path("solo.csv");
        write_comparison_csv(&path, &table, &ExportOptions::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert!(!lines[0].contains("A vs B"));
        // Three key columns plus six metrics.
        assert_eq!(lines[2].split(',').count(), 3 + 6);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_side_leaves_blank_cells_rather_than_shifting_columns() {
        let a = side("A", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B", vec![child(4, 100.0)]);
        let table = table_of(&a, Some(&b), &[]);

        let path = temp_path("ragged.csv");
        write_comparison_csv(&path, &table, &ExportOptions::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        let widths: Vec<usize> = lines.iter().map(|l| l.split(',').count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "every row must have the same column count: {widths:?}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn label_overrides_win_over_derived_labels() {
        let a = side("derived-a", vec![child(4, 200.0)]);
        let b = side("derived-b", vec![child(4, 100.0)]);
        let table = table_of(&a, Some(&b), &[]);

        let options = ExportOptions {
            label_a: Some("Custom A".into()),
            label_b: Some("Custom B".into()),
            ..ExportOptions::default()
        };

        let path = temp_path("labels.csv");
        write_comparison_csv(&path, &table, &options).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        assert!(text.contains("Custom A"));
        assert!(text.contains("Custom B"));
        assert!(!text.contains("derived-a"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn empty_ratio_metrics_fall_back_to_the_default_pair() {
        let options = ExportOptions {
            ratio_metrics: vec![],
            ..ExportOptions::default()
        };
        assert_eq!(
            options.ratios(),
            vec!["output_throughput", "median_tpot_ms"]
        );
    }

    #[test]
    fn selected_metrics_are_reordered_into_the_canonical_order() {
        let options = ExportOptions {
            metric_keys: vec!["output_throughput".into(), "median_ttft_ms".into()],
            ..ExportOptions::default()
        };
        assert_eq!(
            options.metrics(),
            vec!["median_ttft_ms", "output_throughput"]
        );
    }

    #[test]
    fn an_unrecognized_metric_selection_falls_back_to_all_six() {
        let options = ExportOptions {
            metric_keys: vec!["not_a_metric".into()],
            ..ExportOptions::default()
        };
        assert_eq!(options.metrics().len(), 6);
    }

    #[test]
    fn xlsx_writes_a_readable_workbook() {
        let a = side("MI355X", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B200", vec![child(4, 100.0)]);
        let table = table_of(&a, Some(&b), &["tensor_parallel_size".to_string()]);

        let path = temp_path("compare.xlsx");
        write_comparison_xlsx(&path, &table, &ExportOptions::default()).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        // XLSX is a zip archive, so it must start with the local file header.
        assert_eq!(&bytes[..2], b"PK");
        assert!(bytes.len() > 1000, "workbook looks truncated");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn raw_csv_has_one_row_per_child_from_both_sides() {
        let a = side("A", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B", vec![child(4, 100.0)]);
        let none = HashSet::new();

        let path = temp_path("raw.csv");
        write_raw_csv(&path, &[("A", &a, &none), ("B", &b, &none)]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert!(lines[0].starts_with("side,label,run_id,run_name,group,"));
        assert!(lines[0].ends_with("benchmark_yaml_path"));
        assert_eq!(lines.len(), 4); // header + 2 from A + 1 from B
        assert!(lines[1].starts_with("A,"));
        assert!(lines[3].starts_with("B,"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn raw_csv_omits_unticked_children() {
        let a = side("A", vec![child(4, 200.0), child(8, 300.0)]);
        let excluded: HashSet<String> = ["run8".to_string()].into_iter().collect();

        let path = temp_path("raw-filtered.csv");
        write_raw_csv(&path, &[("A", &a, &excluded)]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        assert_eq!(text.lines().count(), 2); // header + the one surviving child
        assert!(text.contains("run4"));
        assert!(!text.contains("run8"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn markdown_names_both_sides_and_emits_a_ratio_per_selected_metric() {
        let a = side("MI355X", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B200", vec![child(4, 100.0), child(8, 150.0)]);
        let table = table_of(&a, Some(&b), &[]);

        let options = ExportOptions {
            metric_keys: vec!["median_tpot_ms".into(), "output_throughput".into()],
            ratio_metrics: vec!["median_tpot_ms".into(), "output_throughput".into()],
            ..ExportOptions::default()
        };
        let text = comparison_markdown(&table, &options);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines[0], "**A** — MI355X");
        assert_eq!(lines[1], "**B** — B200");
        assert_eq!(lines[2], "");

        let header = lines[3];
        assert!(header.starts_with("| ISL | OSL | conc |"));
        assert!(header.contains("A TPOT (ms)"));
        assert!(header.contains("B Output tok/s"));
        assert!(header.contains("A/B TPOT (ms)"));
        assert!(header.contains("A/B Output tok/s"));
        // Only the two selected metrics, on each side, plus their two ratios.
        assert_eq!(header.matches('|').count(), 3 + 2 + 2 + 2 + 1);

        assert_eq!(
            lines[4],
            "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
        );
        // A has double B's throughput at every concurrency.
        assert!(lines[5].contains("200.0%"));
        assert_eq!(lines.len(), 7); // labels, blank, header, rule, two rows
    }

    #[test]
    fn markdown_of_a_single_side_has_no_b_columns_or_ratios() {
        let a = side("Solo", vec![child(4, 200.0)]);
        let table = table_of(&a, None, &[]);

        let text = comparison_markdown(&table, &ExportOptions::default());

        assert!(!text.contains("**B**"));
        assert!(!text.contains("A/B"));
        assert!(!text.contains("| B "));
    }

    // Numbers right, free text left. Getting this wrong is invisible in the raw
    // markdown and only shows up once a forge renders the table.
    #[test]
    fn markdown_left_aligns_comparison_fields_and_right_aligns_numbers() {
        let a = side("MI355X", vec![child(4, 200.0)]);
        let b = side("B200", vec![child(4, 100.0)]);
        let table = table_of(&a, Some(&b), &["tensor_parallel_size".to_string()]);

        let text = comparison_markdown(&table, &ExportOptions::default());
        let alignment = text.lines().nth(4).unwrap();
        let cells: Vec<&str> = alignment
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();

        // ISL, OSL, conc, then each side's field column, its six metrics, then
        // the two default ratio columns.
        let mut expected = vec!["---:"; 3];
        for _ in 0..2 {
            expected.push(":---");
            expected.extend(std::iter::repeat("---:").take(6));
        }
        expected.extend(["---:", "---:"]);

        assert_eq!(cells, expected);
        assert_eq!(
            alignment.matches(":---").count(),
            2,
            "only the two comparison-field columns should be left-aligned"
        );
    }

    #[test]
    fn markdown_escapes_a_pipe_in_a_label() {
        let a = side("left | right", vec![child(4, 200.0)]);
        let table = table_of(&a, None, &[]);

        let text = comparison_markdown(&table, &ExportOptions::default());
        assert!(text.contains("**A** — left \\| right"));
    }
}
