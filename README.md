# rust-hh-feed

This project collects job postings related to the Rust programming language from HeadHunter and posts them to a Telegram channel.
You can join the Telegram channel at [RustHH Jobs](https://t.me/rusthhjobs).

## Main Features

- Read the HeadHunter search RSS feed for fresh `rust` vacancies ordered by publication time.
- Filter vacancies where "Rust" appears in the title.
- Publish the results to a Telegram channel via a bot.
- Run the posting pipeline from GitHub Actions when the HeadHunter workflows are enabled.

## Components

1. **HeadHunter parser** — a Rust module that reads the HeadHunter search RSS feed.
2. **Collector and filter** — processes vacancies and selects relevant ones.
3. **Telegram bot** — sends messages to the channel.
4. **Scheduler** — triggers the collection and posting when the HeadHunter workflows are enabled.

## Documentation
- [Project architecture](docs/README.md)
- [Publication state storage](docs/TECHNICAL_DETAILS.md)

## Configuration
The bot expects a few environment variables:

| Variable | Purpose |
|----------|--------------------------------------------------------------|
| `TELEGRAM_BOT_TOKEN` | Telegram bot token |
| `TELEGRAM_CHAT_ID` | ID of the channel to post jobs |
| `HH_BASE_URL` | Override base URL for the HeadHunter web host or a test double |
| `HH_USER_AGENT` | Override the `User-Agent` value sent to HeadHunter search requests |
| `HH_PROXY_URLS` | Optional comma- or newline-separated proxy URLs for HeadHunter requests only |
| `HH_PROXY_SOURCE_URLS` | Optional comma- or newline-separated URLs that return proxy candidates; if unset, the bot polls built-in public proxy providers |
| `HH_PROXY_PROBE_TIMEOUT_SECS` | Timeout for fetching and probing HeadHunter proxy candidates |
| `BACKFILL_HOURS` | Optional one-off override that widens the fetch window for backfills |
| `TELEGRAM_API_BASE_URL` | Override base URL for the Telegram Bot API |
| `POSTED_JOBS_PATH` | Path to the JSON file with already posted jobs |
| `MANUAL_MODE` | Set to `true` to skip saving posted jobs |
| `JOB_RETENTION_DAYS` | Maximum age in days to keep posted job IDs |

The file referenced by `POSTED_JOBS_PATH` is checkpointed after every acknowledged message in the separate `delivery-state` branch. The fetch watermark advances only after the full batch succeeds, so failed batches retain both confirmed vacancies and their original retry window. The previous artifact is used only for first migration.
The collector now uses the HeadHunter search RSS feed exposed by the public vacancy search page instead of the blocked `/vacancies` API endpoint. `HH_USER_AGENT` remains configurable so production runs can identify themselves consistently.
The bot can dynamically poll several built-in public proxy providers for Russian free proxies when `HH_PROXY_SOURCE_URLS` is unset. If you do define `HH_PROXY_SOURCE_URLS`, those custom sources replace the built-in provider list.
If `HH_PROXY_URLS` is configured or a proxy source returns candidates, the bot probes a bounded candidate set against the HeadHunter search RSS endpoint, keeps the working proxies in order, and retries the real feed request through them until one succeeds before falling back to direct access. Telegram traffic always stays on the direct network path.
For one-off recovery runs you can set `BACKFILL_HOURS`, typically `72`, to fetch missed vacancies from the last three days plus the normal overlap in addition to the state-based window.
The bot always fetches from the last successful committed run with a small overlap window. If the state file is missing or does not contain a committed timestamp yet, it falls back to a wider bootstrap window to reduce the chance of missing vacancies during unstable scheduling.

During continuous integration the workflow sets `TELEGRAM_CHAT_ID` to a development channel.
Scheduled runs and manual releases use the production chat ID.
The HeadHunter posting workflows run on a 20-minute schedule when the GitHub Actions variable `HH_PIPELINES_ENABLED=true`.

Set the `RUST_LOG` environment variable to control the logging level, for
example `RUST_LOG=info`.

Create a `.env` file using [`.env.example`](.env.example) as a template.

## Quiet CI Logs
When running CI workflows you can suppress crate download and compilation
messages by adding `--quiet` to the Cargo commands. For example:

```
cargo clippy --quiet --all-targets --all-features -- -D warnings
cargo test --quiet
cargo run --release --quiet
```

This keeps the logs short while still printing warnings and errors.

## Continuous Integration
Pull requests trigger the [`ci.yml`](.github/workflows/ci.yml) workflow that checks formatting,
lint rules, `cargo machete`, and tests. The `post.yml` workflow
can run the application manually when the HeadHunter workflows are enabled. CI and workflow-security gates validate every PR. Dependabot can enable native auto-merge for its verified current commit when both gates pass. Cargo and Actions updates are batched weekly; toolchain updates are tracked separately.

A push to main rebuilds the latest release binary. The scheduled posting pipeline downloads the release asset. See the dependency policy below for the limitation of merges performed with GITHUB_TOKEN.

Additional workflows automate repository maintenance:

- `pr_cleanup.yml` cancels running CI jobs and deletes the branch after a pull request is merged while skipping its own run.
- `manual_release.yml` allows manual execution of the bot through the GitHub UI when `HH_PIPELINES_ENABLED=true`.
- The `cleanup-old-runs` job inside `post.yml` deletes completed runs of all workflows after three days using `GITHUB_TOKEN` with the `actions: write` permission.


## Release Binary
A push to the `main` branch updates the `latest` release with a freshly built executable. Only one file, `rust-hh-feed`, is kept in the release. Download it with:

```
curl -L https://github.com/<owner>/<repo>/releases/latest/download/rust-hh-feed -o rust-hh-feed
```

## License
This project is licensed under the [MIT](LICENSE) license.

## Delivery checkpoints and recovery

Production delivery stores its checkpoint in the separate `delivery-state` branch using GitHub's contents API. The workflow token is scoped to the restore/send steps and consumed by the checkpoint helper; checkout does not persist credentials. The remote file is a versioned JSON envelope containing base64-encoded application state and a `pending` SHA-256 digest. No token, chat ID, or API response is stored.

Before each send, a pending operation is committed. Only an acknowledged send promotes the application state and clears pending in the same API write. Authentication, network, corrupt-state and checkpoint conflicts stop delivery. A missing branch is the only first-run condition; existing legacy state is migrated where available. A missing file in an existing state branch is an error. Caches and run artifacts are no longer the authority after migration.

A timeout, process crash, or failed checkpoint after an acknowledgement leaves pending set. Subsequent scheduled runs stop before sending. This provides safe interruption, not exactly-once delivery: Telegram has no idempotency key for this operation.

To reconcile:

1. Pause the production workflow and inspect the failed run and Telegram channel. Do not blindly retry or remove state.
2. Read the checkpoint file on `delivery-state` and decode its `state` field locally. If the pending message was delivered, add its confirmed entry to the application state. If it was definitely not delivered, retain the previous application state. If the outcome is still unknown, leave pending intact.
3. Encode the reconciled state, set `pending` to null, and update that same file with its current blob SHA to reject concurrent changes. Keep tokens, chat IDs and response bodies out of the commit. Record the evidence in the recovery change.
4. For local runs, update the local state consistently and remove only its `.pending` sidecar after reconciliation. Resume the workflow. Already confirmed entries are skipped.

## Dependency and workflow policy

Cargo and Actions updates share a weekly Dependabot maintenance batch with a seven-day cooldown; security fixes use a dedicated group. Rust toolchain updates remain tracked separately. Native auto-merge requires the exact PR head to pass `ci-gate` and `workflow-security-gate`, with no review requirement. Failed or skipped required jobs cannot pass a gate. The privileged controller reads metadata only; it never checks out PR code or consumes PR artifacts.

Existing release and delivery triggers are preserved. A merge performed using `GITHUB_TOKEN` does not guarantee a follow-up push workflow. Guaranteed automatic post-merge releases need a separate GitHub App token or verified release controller; this change does not claim that coverage.

The release publishes `delivery-manifest.json` (protocol 1, source commit, SHA-256) alongside the binary. Posting verifies both protocol and hash before execution. Missing old manifests and asset-update races fail closed; wait for the normal release to finish before retrying.

Manual production delivery uses the same reusable posting workflow, backfill input, journal and pending guard as scheduled delivery. It cannot bypass reconciliation.

Migration note (2026-10-14): the retired `auto_merge.yml` trusted an author allowlist including `qqrm`, so its already-running workflow merged the initial migration PR #161 before rollout verification finished. That legacy controller is removed. The replacement enables native auto-merge only for verified Dependabot heads after both gates; ordinary author PRs are not eligible. Manual production delivery and release verification share the guarded path described above.
