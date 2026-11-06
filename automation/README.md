# Deterministic scheduled automations

A read-only Windows Task Scheduler query found no matching GatorGuide/planner task. The repository's weekly GitHub Actions planner workflow is the existing scheduled job and the only active repository-authored workflow.

GitHub may still list platform-managed dynamic workflows in Actions. Dependabot update configuration was removed, Dependabot security auto-updates were disabled, and CodeQL default setup is now not configured. GitHub's Dependency Graph entry is platform-managed (`dynamic/dependabot/update-graph`) and has no repository workflow file; its workflow-disable API request returned HTTP 422.

## Planner refresh

`.github/workflows/planner-refresh.yml` runs weekly on Monday at 12:00 UTC and can also be started manually. Rust is the automation entry point. It calls the maintained planner source pipeline, which still uses the existing deterministic Node.js/PDF/source adapters rather than replacing that large, tested parser in one risky rewrite. Automatic parser repair is disabled for scheduled runs.

For successful scheduled/manual refreshes, the workflow stages only public planner outputs, commits and pushes changed data to `main`, and creates a uniquely tagged GitHub release even when the data is unchanged. It also builds and deploys GitHub Pages. Direct pushes to `main` use the same workflow for Pages deployment without running a planner refresh or creating a release.

## Scholarship and internship source review

`.github/workflows/scholarship-refresh.yml` and `internship-refresh.yml` define weekly schedules and manual dispatch, but are deliberately disabled in the repository's Actions settings. They are not run until explicitly enabled. Their source registry is the matching category in `source/data/starter-opportunities.json`; each source is the existing `source.sourceUrl`, falling back to `externalUrl` only when needed.

The Rust collector makes bounded HTTPS requests to those curated URLs, parses page metadata, Schema.org JSON-LD and matching category links deterministically, then uploads a review-only JSON artifact. It does not use AI, crawl discovered links, edit the public catalog, commit, push, or create releases. Candidate facts must be reviewed before they become shared records in the catalog JSON.

Enable a category workflow only when ready:

```sh
gh workflow enable "Scholarship Source Review (Dormant)" --repo MarsLuay/GatorGuide
gh workflow enable "Internship Source Review (Dormant)" --repo MarsLuay/GatorGuide
```

Run the collector locally without changing the catalog:

```sh
cargo run --locked --release --manifest-path automation/Cargo.toml -- \
  collect-opportunities --category scholarships --output /tmp/scholarship-candidates.json

cargo run --locked --release --manifest-path automation/Cargo.toml -- \
  collect-opportunities --category internships --output /tmp/internship-candidates.json
```
