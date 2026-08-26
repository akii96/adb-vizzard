# Design notes

Context for maintainers: how the app is put together and the decisions that are
easy to get wrong. For usage, see the [README](../README.md).

## Shape

A Tauri 2 app: React frontend, Rust backend, no server.

```
src/                    React UI
  components/           boot dialog, run pickers, table, chart, exports
  store/session.ts      all UI state (Zustand)
  lib/ipc.ts            typed wrappers over the Tauri commands
src-tauri/src/
  commands.rs           the IPC surface
  client.rs             MLflow REST client, retries, credential vending
  fetcher.rs            parallel fetch; remote and local-folder sources
  cache.rs              in-memory session cache
  parser.rs             commands.txt and benchmark JSON
  comparator.rs         the join, ratios, Pareto frontier
  exporter.rs           CSV and XLSX writers
  creds.rs              credentials, masking, zeroize
  settings.rs           settings file and boot credential precedence
```

All computation — parsing, joining, ratios, sorting — happens in Rust. The
frontend receives a flat array and renders it. Keep it that way; it is what makes
a wide table stay responsive.

## How a load works

```
runs/get  →  runs/search (children)  →  discover harness dir
          →  credentials-for-read (batched, per run)
          →  parallel GETs straight from Azure Blob
          →  parse + join in Rust  →  one array to the UI
```

The design driver is that **loading is latency-bound, not bandwidth-bound**. A
child run contributes about 3.5 KB, so a 22-child sweep is roughly 77 KB. The wins
come from removing round trips, not from moving bytes faster:

- Ask for the two artifact paths by name instead of walking the artifact tree.
- Vend signed URIs in one batched `credentials-for-read` call per run, then fetch
  bytes directly from blob storage, keeping the Databricks control plane out of the
  data path.
- Fan out across children over one pooled, keep-alive HTTP client.

Measured on a 22-child sweep over a corporate proxy: **4.4 s cold, 2.4 s warm**.
Control-plane round trips cost ~500 ms each here and four are needed before any
artifact is touched, so ~2 s is the floor. The cache removes the blob GETs, which
is the cold/warm difference. Making warm loads faster would mean caching run
*metadata*, which trades staleness for speed — deliberately not done.

`cargo run --example live_load -- <run_id>` exercises this path and prints timings.

## Things that will bite you

**Two benchmark harnesses produce the same artifact, and they disagree.**
`vllm bench serve` and `sglang.bench_serving` differ in three ways, each of which
broke this app independently:

| | vllm | sglang |
|---|---|---|
| Directory | `benchmark_results/0_vllm_bench_serve` | `benchmark_results/0_sglang_bench_serve` |
| End-to-end latency | `median_e2el_ms` | `median_e2e_latency_ms` |
| Total throughput | `total_token_throughput` | `total_throughput` |

So the harness directory is **discovered**, never assumed, and metrics are read
through an alias list. Both variants are covered by fixtures.

**The sglang artifact is not valid JSON.** It contains `"request_rate": Infinity`,
a bare literal that RFC 8259 disallows. Python's `json` accepts it as an extension,
so the CLI never noticed; `serde_json` rejects the whole document. Non-finite
literals are rewritten to `null` before parsing.

**Rounding has to match Python.** Metrics go through `parser::round2`, which
mirrors Python's `round()`. The obvious `(v * 100.0).round() / 100.0` is wrong
twice over — it rounds halves away from zero instead of to even, and the multiply
itself double-rounds, so `2.675` comes out `2.68` where Python gives `2.67`. Tests
pin this.

**The file named `yaml` contains JSON.** Upstream's naming, not a mistake. A test
pins it so nobody "fixes" it into a YAML parser.

**Parent directories are not reliably prefixed `parent_`.** `adb-pull` writes
`parent_<slug>_<hash>`, but real pulls get renamed. Local-folder mode finds parents
by looking for `child_*` subdirectories.

**TLS must use the Windows certificate store.** `reqwest` is configured with
`native-tls` (Schannel), not rustls with bundled roots. Corporate networks
terminate TLS at an inspection proxy and re-sign with a private root that is
installed in Windows but absent from `webpki-roots`, so bundled roots cannot reach
any workspace from inside the network while `curl` and Edge can. If you ever move
to rustls, use `rustls-tls-native-roots`.

## Databricks API notes

- `POST /api/2.0/mlflow/artifacts/credentials-for-read` is undocumented but is what
  the official client uses. It takes a run ID plus a list of paths and returns one
  `AZURE_SAS_URI` each.
- It **signs paths without checking they exist**, so it cannot be used to probe for
  an artifact — the URI will happily 404 later. That is why harness discovery uses
  `artifacts/list`.
- `GET /api/2.0/mlflow/artifacts/get` does **not** exist on Databricks
  (`ENDPOINT_NOT_FOUND`). The code keeps it for self-hosted MLflow, but it is not a
  usable fallback here.
- The control plane rate-limits (`RESOURCE_EXHAUSTED`); the 429 backoff covers it.

## Secrets

The token lives in `SessionCreds` on the Rust side and reaches the frontend only
through the explicit `reveal_token` command. Everything user-visible goes through
`Secrets::redact` first, because `reqwest` and URL errors echo back whatever you
handed them. `Debug`/`Display` on credentials print `***`.

Artifacts are **never written to disk**. `commands.txt` embeds the docker `-e`
environment, which in real pulls includes `HF_TOKEN`, so a disk cache would leave
third-party credentials lying around. Buffers are zeroized when dropped. This is
also why fixtures must be scrubbed — see [fixtures/README.md](../fixtures/README.md).

## Packaging

NSIS per-user install (`installMode: "currentUser"`) so no admin rights are needed,
which is what usually blocks a utility on a managed laptop. Note that
`installMode: "both"` would force an admin prompt even when the user picks per-user,
so it is not offered. `embedBootstrapper` costs ~1.8 MB but survives a proxy that
blocks the WebView2 CDN.

Charts and the XLSX writer load via dynamic `import()`, so they are not in the boot
bundle; ECharts is imported from `echarts/core` with only the used components
registered.

Settings live under `%APPDATA%\<bundle identifier>\`, which Tauri derives from
`identifier` in `tauri.conf.json` — not from `productName`. The app reports the
real path to the UI rather than hardcoding it.

## Not in v1

- Docker log download and the `adb-fallbacks` extraction workflow. The fetch
  pipeline generalises to it: add a non-recursive root listing to find the
  hash-named logs, and range-chunked GETs for large ones.
- More than two sides in a comparison.
- Persisting anything beyond settings.
