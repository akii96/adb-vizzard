# Contributing

## Setup

Needs Node 18+ and Rust stable. On Windows you also need the MSVC build tools for
the linker:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Then:

```bash
npm install
npm run icons        # generates src-tauri/icons
npm run tauri dev
```

## Before a PR

CI runs exactly this, and treats clippy warnings as errors:

```bash
npm run typecheck
npx vite build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Tests include golden-file tests that run the real parsers over the committed
fixtures, offline. They are what keep our numbers matching `adb-summarize`, so if
you change output formatting they need to pass.

To check against a live workspace (needs credentials, so it is not a test):

```bash
cargo run --example live_load --manifest-path src-tauri/Cargo.toml -- <run_id>
```

## Conventions

Read [docs/DESIGN.md](docs/DESIGN.md) first — it lists the non-obvious constraints,
several of which look like bugs and are not.

The short version:

- **Computation belongs in Rust.** The frontend renders; it does not calculate.
- **The token must not leak.** It reaches the UI only via `reveal_token`, and
  anything user-visible goes through `Secrets::redact`.
- **Artifacts stay in memory.** Do not add a disk cache; `commands.txt` carries
  third-party tokens.
- **Match the CLI's numbers.** Rounding goes through `parser::round2`.

## Fixtures

See [fixtures/README.md](fixtures/README.md). **Scrub secrets before committing** —
real `commands.txt` files contain live tokens.

## Releasing

Bump `version` in both `package.json` and `src-tauri/tauri.conf.json`, commit, then:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The release workflow checks the tag matches the app version, runs the tests, builds
the installers plus a portable zip, and publishes them.
