# ADB Vizzard

A desktop app for viewing and comparing Azure Databricks MLflow benchmark sweeps.
Paste one or two run IDs and get the comparison table, performance curves, and
CSV/XLSX exports — no `adb-pull` / `adb-summarize` round trip.

## Install

Download from the [latest release](../../releases/latest) and run it. **Nothing
needs installing first** — no Python, Node, or Rust.

| Download | Use it when |
|---|---|
| `ADB Vizzard_*_x64-setup.exe` | Normal case. Installs per-user, **no admin rights needed**. |
| `ADB Vizzard_*_x64_en-US.msi` | IT-managed, per-machine rollout. |
| `ADB Vizzard_*_x64_portable.zip` | No install at all — unzip and run. |

Sharing with a colleague is just sending them one of these files.

## Using it

**1. Connect.** Enter your workspace host and a personal access token.

Tick *Remember me on this PC* to skip this next time. If you already have
`DATABRICKS_HOST` and `DATABRICKS_TOKEN` in your environment, or a `.env` beside
the app, they are picked up automatically and connecting is one click.

**2. Load a run.** Paste an MLflow run ID *or* a Databricks run URL into the left
panel and press Enter. A parent run expands into its child runs; a single run is
treated as one case. Untick any cases you want to exclude.

**3. Compare.** Load a second run on the right. Rows line up on
`(input length, output length, concurrency)`, so the same benchmark case appears
side by side with a ratio column. Cases present on only one side still get a row,
marked rather than dropped.

**4. Explore.** The *Curves* tab plots any metric against concurrency, input
length, or output length. Hovering a point tells you which case it is
(`ISL 1000 / OSL 100 / mc 16`). Points on the Pareto frontier are ringed. Drag to
zoom, double-click to reset.

**5. Export.** Comparison XLSX with merged headers, comparison CSV, a flat
per-run CSV, or the chart as a PNG. Column names and rounding match
`adb-summarize`, so exports drop straight into existing spreadsheets. Side labels
are auto-derived and editable before export.

### Shortcuts

| Key | Action |
|---|---|
| `Ctrl+L` / `Ctrl+R` | Focus the left / right run field |
| `Enter` | Load that side |

### Without credentials

*Open a local pull folder* reads an existing `exp_pull_*` directory — useful if
you already ran `adb-pull` or a colleague shared a pull. You can get there
straight from the login screen via **Skip — work from a local pull folder**.

Point each side at a *parent* directory (the one containing `child_*` folders) to
compare two pulls.

## Credentials and data

Your token is masked in the UI and only shown if you click the eye icon. It is
kept out of error messages and cleared when you disconnect.

If you tick *Remember me*, the token is saved **in plain text** to a settings file
under `%APPDATA%`; the exact path is shown in Settings. Anything running as your
Windows user could read it. It is off by default, and *Settings → Forget* clears
it. If your settings folder is inside OneDrive or similar, only the host is saved.

Benchmark data is held in memory while the app is open and never written to disk,
so reopening a run is fast and nothing lingers afterwards.

## If something goes wrong

**Can't reach the host.** TLS trust comes from the Windows certificate store, so
networks that inspect TLS work without setup. If the same URL loads in Edge but
not here, the error message will say whether it looks like a certificate problem.

**A run loads with some cases missing.** Cases that could not be read are listed
under the run with the reason. The rest still load.

**Loads feel slow.** Most of the time is workspace API round trips, not data
volume. Reopening the same run in one session is much faster. Concurrency limits
are adjustable in Settings if you are on a slow VPN.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, and
[docs/DESIGN.md](docs/DESIGN.md) for how it works and why.

Licensed under the [MIT License](LICENSE).
