//! Export writers.
//!
//! Three outputs: the comparison table with the two-row grouped header the
//! example CSV uses, the same thing as XLSX with real merged cells, and a flat
//! per-child CSV that stays column-compatible with `adb-summarize`.

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
    ratio_metrics: Vec<String>,
    /// Columns per side: comparison fields, then the six metrics.
    side_span: usize,
    has_b: bool,
}

impl Layout {
    fn new(table: &ComparisonTable, options: &ExportOptions) -> Self {
        let field_headers = table.field_headers.clone();
        let side_span = field_headers.len() + METRIC_KEYS.len();
        Self {
            field_headers,
            ratio_metrics: options.ratios(),
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
            row.extend(METRIC_KEYS.iter().map(|m| m.to_string()));
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
fn metric_values(cell: Option<&CellGroup>) -> Vec<Option<f64>> {
    match cell {
        Some(cell) => METRIC_KEYS
            .iter()
            .map(|key| cell.metrics.get(key))
            .collect(),
        None => vec![None; METRIC_KEYS.len()],
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
                metric_values(cell)
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

            for value in metric_values(cell) {
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

/// Writes the flat per-child CSV.
///
/// Column set matches `adb-summarize`'s summary output plus the side and run
/// identity, and carries `benchmark_yaml_path` last for traceability.
pub fn write_raw_csv(path: &Path, sides: &[(&str, &SideData)]) -> AppResult<()> {
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

    for (side_name, side) in sides {
        for child in &side.children {
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
        let table = build_table(&a, Some(&b), &fields, Aggregation::Median);

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
        let table = build_table(&a, None, &[], Aggregation::Median);

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
        let table = build_table(&a, Some(&b), &[], Aggregation::Median);

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
        let table = build_table(&a, Some(&b), &[], Aggregation::Median);

        let options = ExportOptions {
            label_a: Some("Custom A".into()),
            label_b: Some("Custom B".into()),
            ratio_metrics: vec![],
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
            label_a: None,
            label_b: None,
            ratio_metrics: vec![],
        };
        assert_eq!(
            options.ratios(),
            vec!["output_throughput", "median_tpot_ms"]
        );
    }

    #[test]
    fn xlsx_writes_a_readable_workbook() {
        let a = side("MI355X", vec![child(4, 200.0), child(8, 300.0)]);
        let b = side("B200", vec![child(4, 100.0)]);
        let table = build_table(
            &a,
            Some(&b),
            &["tensor_parallel_size".to_string()],
            Aggregation::Median,
        );

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

        let path = temp_path("raw.csv");
        write_raw_csv(&path, &[("A", &a), ("B", &b)]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert!(lines[0].starts_with("side,label,run_id,run_name,group,"));
        assert!(lines[0].ends_with("benchmark_yaml_path"));
        assert_eq!(lines.len(), 4); // header + 2 from A + 1 from B
        assert!(lines[1].starts_with("A,"));
        assert!(lines[3].starts_with("B,"));

        let _ = std::fs::remove_file(path);
    }
}
