use serde::Serialize;
use std::fs;
use std::path::Path;

pub fn serialize_json_report<T: Serialize>(report: &T) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map(|json| format!("{json}\n"))
        .map_err(|error| format!("could not serialize JSON report: {error}"))
}

pub fn serialize_markdown_lines(lines: &[String]) -> String {
    format!("{}\n", lines.join("\n"))
}

pub fn normalize_markdown_report(markdown: &str) -> String {
    if markdown.ends_with('\n') {
        markdown.to_string()
    } else {
        format!("{markdown}\n")
    }
}

pub fn write_json_report<T: Serialize>(path: &Path, report: &T) -> Result<(), String> {
    create_parent_directory(path)?;
    let output = serialize_json_report(report)?;
    fs::write(path, output)
        .map_err(|error| format!("could not write JSON report {}: {error}", path.display()))
}

pub fn write_markdown_report(path: &Path, markdown: &str) -> Result<(), String> {
    create_parent_directory(path)?;
    fs::write(path, normalize_markdown_report(markdown)).map_err(|error| {
        format!(
            "could not write Markdown report {}: {error}",
            path.display()
        )
    })
}

pub fn write_markdown_lines_report(path: &Path, lines: &[String]) -> Result<(), String> {
    create_parent_directory(path)?;
    fs::write(path, serialize_markdown_lines(lines)).map_err(|error| {
        format!(
            "could not write Markdown report {}: {error}",
            path.display()
        )
    })
}

pub fn write_report_pair<T: Serialize>(
    json_path: &Path,
    markdown_path: &Path,
    report: &T,
    markdown: &str,
) -> Result<(), String> {
    write_json_report(json_path, report)?;
    write_markdown_report(markdown_path, markdown)
}

fn create_parent_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "could not create report directory {}: {error}",
            parent.display()
        )
    })
}
