# Test fixtures

Real artifacts from `adb-pull` runs, trimmed to the two files per child run that
the app reads. Used by the golden-file tests in
[`src-tauri/tests/golden.rs`](../src-tauri/tests/golden.rs), which run with no
network and no credentials.

There is one pull per benchmark harness, because the two differ in ways that have
each broken the app before:

```
exp_pull_20260325_103811/            # vllm bench serve
├── AMD-GPT-TP1-.../child_.../{commands.txt, benchmark_results/0_vllm_bench_serve/yaml}
└── OAI-GPT-TP1-.../child_.../{commands.txt, benchmark_results/0_vllm_bench_serve/yaml}

exp_pull_sglang_oob_vllm_tp4_nvfp4/  # sglang.bench_serving
└── oob_vllm_tp4_nvfp4/child_.../{commands.txt, benchmark_results/0_sglang_bench_serve/yaml}
```

Keep both. See [docs/DESIGN.md](../docs/DESIGN.md) for what differs between them.

## Adding fixtures

**Scrub secrets first.** Real `commands.txt` files carry a live `HF_TOKEN` in their
`-e` lines; the ones here have it replaced with `hf_REDACTED_FOR_FIXTURE`, and a
test asserts that. Check any new file before committing.

Keep them small — only `commands.txt` and the benchmark `yaml`. Do not add
`docker_container_*.log`, `traces*/`, or `reports/`; they run to megabytes and
nothing reads them.
