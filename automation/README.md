# Deterministic scheduled automations

A read-only Windows Task Scheduler query found no matching GatorGuide/planner task. The repository has an active weekly planner workflow and a dedicated monthly UW Green River transfer-equivalency refresh; scholarship and internship review workflows remain disabled.

GitHub may still list platform-managed dynamic workflows in Actions. Dependabot update configuration was removed, Dependabot security auto-updates were disabled, and CodeQL default setup is now not configured. GitHub's Dependency Graph entry is platform-managed (`dynamic/dependabot/update-graph`) and has no repository workflow file; its workflow-disable API request returned HTTP 422.

## Planner refresh

`.github/workflows/planner-refresh.yml` runs weekly on Monday at 12:00 UTC and can also be started manually. Rust is the automation entry point. The Rust crate now handles bounded Green River public-material discovery, annual-schedule link parsing and recent-year filtering, and current-catalog link parsing, catalog pagination and course-heading/body parsing, course credit/description extraction, labeled enrollment/corequisite/satisfies-requirement section parsing, linked enrollment-anchor extraction and context-filtered course-code candidates, enrollment and corequisite AND/OR path parsing with slash shorthand, punctuation/symbol conjunctions, balanced and nested alternatives, grade-context text, and concurrent-enrollment paths. The Node adapter retains a fallback for expressions the Rust parser cannot safely classify. The main degree-requirement parser also delegates batched inline sequence-alternative segmentation and subject-aware carried-course-number expansion for lettered “choose one option” groups to Rust; Node supplies validated explicit-course candidates, constructs the compatibility output, and retains fallback behavior. Rust also handles general-education requirement classification; UW catalog ingestion; official deadline parsing/catalog updates; UW-GRC equivalency-guide row parsing and base rule generation; support-rule resolution; structured/chain derived-rule synthesis; deterministic chunked TypeScript rendering; and equivalency report generation. The Node adapter supplies the local GRC course-title map and declarative derivation specs, then writes Rust-rendered artifacts and reports. The main degree-requirement source parser, Green River catalog fetch/orchestration, and other planner generators still use Node.js/PDF implementations and remain to be migrated; scheduled parser repair is disabled.

For successful scheduled/manual refreshes, the workflow stages only public planner outputs, commits and pushes changed data to `main`, and creates a uniquely tagged GitHub release even when the data is unchanged. It also builds and deploys GitHub Pages. Direct pushes to `main` use the same workflow for Pages deployment without running a planner refresh or creating a release.

## Monthly Transfer Category Equivalencies refresh

`.github/workflows/transfer-category-equivalencies-monthly.yml` runs on the first day of each month at 12:00 UTC and can also be started manually. It parses the UW Green River Equivalency Guide and regenerates both the planner guide rules and the lightweight Transfer Category Equivalencies catalog. When either public generated file changes, the workflow commits and pushes only those outputs to `main`; the existing Pages workflow handles deployment. The resource copy is checked for “Double check using Equivalency Guide.” and the intentionally blank “Last updated at ____.” placeholder on each run. The broader weekly planner refresh also runs the guide parser as part of its full data refresh.

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
