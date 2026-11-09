use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EQUIVALENCY_OUTPUTS: &[&str] = &[
    "source/constants/transfer-planner-source/equivalency-guide.generated.ts",
    "source/constants/transfer-equivalency-catalog.generated.ts",
    "source/constants/locales/en.json",
];
const PLANNER_OUTPUTS: &[&str] = &[
    "source/data/starter-opportunities.json",
    "source/constants/green-river-major-options.generated.ts",
    "source/constants/transfer-equivalency-catalog.generated.ts",
    "source/constants/transfer-planner-grc-availability.generated.ts",
    "source/constants/transfer-planner-source/",
    "source/docs/planner/",
];
const ACTIONS_BOT_NAME: &str = "github-actions[bot]";
const ACTIONS_BOT_EMAIL: &str = "41898282+github-actions[bot]@users.noreply.github.com";

fn main() {
    if let Err(error) = run() {
        eprintln!("workflow action failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = env::args()
        .nth(1)
        .ok_or_else(|| "missing workflow-actions command".to_string())?;
    match command.as_str() {
        "publish-equivalencies" => publish_equivalencies(),
        "publish-planner" => publish_planner(),
        "copy-pages-fallback" => copy_pages_fallback(),
        "--help" | "-h" | "help" => {
            println!("Commands: publish-equivalencies, publish-planner, copy-pages-fallback");
            Ok(())
        }
        other => Err(format!("unknown workflow-actions command {other:?}")),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workflow-actions crate must be under automation/")
        .to_path_buf()
}

fn publish_equivalencies() -> Result<(), String> {
    publish_changed_paths(
        EQUIVALENCY_OUTPUTS,
        "chore(equivalencies): monthly UW guide refresh",
    )
}

fn publish_planner() -> Result<(), String> {
    publish_changed_paths(
        PLANNER_OUTPUTS,
        "chore(planner): refresh official source data",
    )?;
    run_git(&["fetch", "origin", "main"])?;
    let target_output = git_output(&["rev-parse", "origin/main"])?;
    if !target_output.status.success() {
        return Err(command_failure(
            "git",
            &["rev-parse", "origin/main"],
            &target_output,
        ));
    }
    let target = String::from_utf8_lossy(&target_output.stdout)
        .trim()
        .to_string();
    if target.is_empty() {
        return Err("git returned an empty release target for origin/main".into());
    }

    let run_id = required_env("GITHUB_RUN_ID")?;
    let run_attempt = required_env("GITHUB_RUN_ATTEMPT")?;
    let run_number = required_env("GITHUB_RUN_NUMBER")?;
    let tag = format!("planner-refresh-{run_id}-{run_attempt}");
    run_command(
        "gh",
        &[
            "release",
            "create",
            &tag,
            "--target",
            &target,
            "--title",
            &format!("Planner refresh {run_number}"),
            "--notes",
            "Automated weekly planner refresh. Refreshed public data is committed to main when source outputs change.",
        ],
        &repo_root(),
    )
}

fn publish_changed_paths(paths: &[&str], commit_message: &str) -> Result<(), String> {
    run_git(&["config", "user.name", ACTIONS_BOT_NAME])?;
    run_git(&["config", "user.email", ACTIONS_BOT_EMAIL])?;

    let mut add_args = vec!["add", "--"];
    add_args.extend(paths.iter().copied());
    run_git(&add_args)?;
    run_git(&["diff", "--cached", "--check"])?;

    let staged_changes = git_output(&["diff", "--cached", "--quiet"])?;
    match staged_changes.status.code() {
        Some(0) => {
            println!("Generated data is unchanged; no data commit is needed.");
            Ok(())
        }
        Some(1) => {
            run_git(&["commit", "-m", commit_message])?;
            if git_output(&["push", "origin", "HEAD:main"])?
                .status
                .success()
            {
                return Ok(());
            }
            run_git(&["fetch", "origin", "main"])?;
            run_git(&["merge", "--no-edit", "origin/main"])?;
            run_git(&["push", "origin", "HEAD:main"])?;
            Ok(())
        }
        code => Err(format!(
            "git diff --cached --quiet returned unexpected status {code:?}"
        )),
    }
}

fn copy_pages_fallback() -> Result<(), String> {
    let source = repo_root().join("source/dist/index.html");
    let target = repo_root().join("source/dist/404.html");
    if !source.is_file() {
        return Err(format!("web export is missing {}", source.display()));
    }
    fs::copy(&source, &target).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    println!("Created {} for GitHub Pages routing.", target.display());
    Ok(())
}

fn required_env(name: &str) -> Result<String, String> {
    env::var(name)
        .map_err(|_| format!("required GitHub Actions environment variable {name} is missing"))
}

fn git_output(args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(repo_root())
        .output()
        .map_err(|error| format!("could not start git {}: {error}", args.join(" ")))
}

fn run_git(args: &[&str]) -> Result<(), String> {
    let output = git_output(args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("git", args, &output))
    }
}

fn run_command(program: &str, args: &[&str], cwd: &Path) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("could not start {program} {}: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure(program, args, &output))
    }
}

fn command_failure(program: &str, args: &[&str], output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    format!(
        "{program} {} failed with status {:?}{}{}",
        args.join(" "),
        output.status.code(),
        if stderr.is_empty() { "" } else { ": " },
        if stderr.is_empty() { stdout } else { stderr }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalency_publish_stages_only_refresh_outputs() {
        assert_eq!(
            EQUIVALENCY_OUTPUTS,
            &[
                "source/constants/transfer-planner-source/equivalency-guide.generated.ts",
                "source/constants/transfer-equivalency-catalog.generated.ts",
                "source/constants/locales/en.json",
            ]
        );
    }

    #[test]
    fn planner_publish_stages_only_the_existing_refresh_allowlist() {
        assert_eq!(
            PLANNER_OUTPUTS,
            &[
                "source/data/starter-opportunities.json",
                "source/constants/green-river-major-options.generated.ts",
                "source/constants/transfer-equivalency-catalog.generated.ts",
                "source/constants/transfer-planner-grc-availability.generated.ts",
                "source/constants/transfer-planner-source/",
                "source/docs/planner/",
            ]
        );
    }

    #[test]
    fn push_deployment_skips_planner_preflight_but_checks_pages_actions() {
        let workflow_path = repo_root().join(".github/workflows/planner-refresh.yml");
        let planner = fs::read_to_string(workflow_path).unwrap();
        let planner_check = planner
            .find("- name: Check planner Rust automation")
            .unwrap();
        let workflow_actions_check = planner.find("- name: Check workflow actions").unwrap();
        let refresh = planner.find("- name: Refresh planner data").unwrap();
        let export = planner.find("- name: Export static web app").unwrap();

        assert!(planner_check < workflow_actions_check);
        assert!(workflow_actions_check < refresh);
        assert!(refresh < export);

        let planner_preflight = &planner[planner_check..workflow_actions_check];
        assert!(planner_preflight.contains("if: github.event_name != 'push'"));
        assert!(
            planner_preflight
                .contains("cargo fmt --manifest-path automation/Cargo.toml -- --check")
        );
        assert!(
            planner_preflight.contains("cargo test --locked --manifest-path automation/Cargo.toml")
        );

        let workflow_actions_preflight = &planner[workflow_actions_check..refresh];
        assert!(workflow_actions_preflight.contains(
            "cargo fmt --manifest-path automation/workflow-actions/Cargo.toml -- --check"
        ));
        assert!(workflow_actions_preflight.contains(
            "cargo test --locked --manifest-path automation/workflow-actions/Cargo.toml"
        ));
        assert!(workflow_actions_preflight.contains(
            "cargo build --locked --release --manifest-path automation/workflow-actions/Cargo.toml"
        ));
        assert!(!workflow_actions_preflight.contains("github.event_name != 'push'"));
    }

    #[test]
    fn scheduled_workflows_use_rust_for_custom_refresh_publish_and_pages_steps() {
        let workflows = repo_root().join(".github/workflows");
        let equivalencies =
            fs::read_to_string(workflows.join("transfer-category-equivalencies-monthly.yml"))
                .unwrap();
        let planner = fs::read_to_string(workflows.join("planner-refresh.yml")).unwrap();
        let scholarships = fs::read_to_string(workflows.join("scholarship-refresh.yml")).unwrap();
        let internships = fs::read_to_string(workflows.join("internship-refresh.yml")).unwrap();

        assert!(equivalencies.contains("refresh-equivalencies"));
        assert!(equivalencies.contains("publish-equivalencies"));
        assert!(!equivalencies.contains("parse-transfer-planner-equivalency-guide.cjs"));
        assert!(!equivalencies.contains("node "));
        assert!(!equivalencies.contains("npm "));
        assert!(!equivalencies.contains("shell: bash"));

        assert!(planner.contains("refresh-planner"));
        assert!(planner.contains("publish-planner"));
        assert!(planner.contains("copy-pages-fallback"));
        assert!(!planner.contains("shell: bash"));
        assert!(!planner.contains("git add"));
        assert!(!planner.contains("gh release create"));
        assert!(!planner.contains("cp dist/index.html"));

        assert!(scholarships.contains("collect-opportunities --category scholarships"));
        assert!(internships.contains("collect-opportunities --category internships"));
    }
}
