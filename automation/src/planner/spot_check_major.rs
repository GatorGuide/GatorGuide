use crate::planner::write_markdown_report;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const BOOTSTRAP_EXPORT: &str = "TRANSFER_PLANNER_BOOTSTRAP_ALL_MAJOR_PLANS";
const COMMAND_FILE_NAME: &str = "planner-major-spot-check-commands.md";
const VALID_CAMPUSES: &[&str] = &["uw-bothell", "uw-seattle", "uw-tacoma"];

#[derive(Clone, Debug, PartialEq)]
struct Major {
    plan_id: Option<Value>,
    campus_id: Option<Value>,
    title: Option<Value>,
    pathway_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SpotCheckOptions {
    campus_id: Option<String>,
    plan_id: Option<String>,
    list: bool,
    commands: bool,
    write_command_file: bool,
    write_path: Option<String>,
    refresh: bool,
    parse: bool,
    assert: bool,
    probe: bool,
}

pub fn run_spot_check_major(repository_root: &Path, args: &[String]) -> Result<bool, String> {
    if has_arg(args, &["--help", "-h"]) {
        print_usage();
        return Ok(true);
    }

    let options = parse_options(args);
    let source_root = repository_root.join("source");
    let bootstrap_path =
        source_root.join("constants/transfer-planner-source/bootstrap.generated.ts");
    let source = fs::read_to_string(&bootstrap_path).map_err(|error| {
        format!(
            "could not read planner bootstrap {}: {error}",
            bootstrap_path.display()
        )
    })?;
    let plans = parse_top_level_json_array(
        &source,
        BOOTSTRAP_EXPORT,
        &bootstrap_path.display().to_string(),
    )?;
    let all_majors = planner_majors(&plans, None);

    if options.list {
        print_major_list(&planner_majors(&plans, options.campus_id.as_deref()));
        return Ok(true);
    }

    if options.commands {
        let majors = planner_majors(&plans, options.campus_id.as_deref());
        let write_path = if options.write_command_file {
            let requested = options
                .write_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(".tmp")
                        .join("reports")
                        .join(COMMAND_FILE_NAME)
                });
            Some(resolve_path(&source_root, &requested))
        } else {
            None
        };
        print_commands(&majors, write_path.as_deref())?;
        return Ok(true);
    }

    let Some(plan_id) = options.plan_id.as_deref() else {
        print_usage();
        return Ok(false);
    };
    let Some(major) = all_majors
        .iter()
        .find(|major| major.plan_id.as_ref() == Some(&Value::String(plan_id.to_string())))
    else {
        return Err(format!(
            "No planner major found for {plan_id}. Run --list to see available plan ids."
        ));
    };

    if !options.refresh && !options.parse && !options.assert && !options.probe {
        println!(
            "{}\t{}\t{}",
            js_string(major.plan_id.as_ref()),
            js_string(major.campus_id.as_ref()),
            js_string(major.title.as_ref())
        );
        println!();
        println!("Direct commands:");
        for command in direct_commands(&js_string(major.plan_id.as_ref())) {
            println!("{command}");
        }
        return Ok(true);
    }

    run_selected_checks(&source_root, plan_id, &options)
}

fn parse_top_level_json_array(
    source: &str,
    export_name: &str,
    file_path: &str,
) -> Result<Vec<Value>, String> {
    let declaration = format!("export const {export_name}");
    let Some(export_index) = source.find(&declaration) else {
        return Err(format!("Could not find {export_name} in {file_path}."));
    };
    let search = &source[export_index..];
    let Some(equals_offset) = search.find('=') else {
        return Err(format!("Could not find initializer for {export_name}."));
    };
    let initializer = &search[equals_offset..];
    let Some(array_offset) = initializer.find('[') else {
        return Err(format!(
            "Could not find array initializer for {export_name}."
        ));
    };
    let array_start = export_index + equals_offset + array_offset;
    let Some(array_end) = find_json_array_end(source.as_bytes(), array_start) else {
        return Err(format!(
            "Could not parse array initializer for {export_name}."
        ));
    };
    serde_json::from_str(&source[array_start..array_end])
        .map_err(|error| format!("Could not parse array initializer for {export_name}: {error}."))
}

fn find_json_array_end(source: &[u8], start: usize) -> Option<usize> {
    if source.get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in source.iter().copied().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn planner_majors(plans: &[Value], campus_filter: Option<&str>) -> Vec<Major> {
    let mut plans_by_campus_and_title: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, plan) in plans.iter().enumerate() {
        let key = format!(
            "{}::{}",
            js_string(plan.get("campusId")),
            normalize_alias_text(plan.get("title"))
        );
        plans_by_campus_and_title
            .entry(key)
            .or_default()
            .push(index);
    }

    let mut majors = plans
        .iter()
        .enumerate()
        .filter(|(_, plan)| {
            let alias_parts = get_alias_title_parts(plan);
            let Some((parent_title, _)) = alias_parts else {
                return true;
            };
            let parent_key = format!(
                "{}::{}",
                js_string(plan.get("campusId")),
                normalize_alias_text(Some(&Value::String(parent_title)))
            );
            !plans_by_campus_and_title
                .get(&parent_key)
                .is_some_and(|candidate_indexes| {
                    candidate_indexes
                        .iter()
                        .any(|candidate_index| plans[*candidate_index].get("id") != plan.get("id"))
                })
        })
        .filter(|(_, plan)| {
            campus_filter.is_none_or(|campus| {
                plan.get("campusId") == Some(&Value::String(campus.to_string()))
            })
        })
        .map(|(_, plan)| Major {
            plan_id: plan.get("id").cloned(),
            campus_id: plan.get("campusId").cloned(),
            title: plan.get("title").cloned(),
            pathway_count: plan
                .get("pathways")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        })
        .collect::<Vec<_>>();
    majors.sort_by(|left, right| {
        js_locale_compare(
            &js_string(left.campus_id.as_ref()),
            &js_string(right.campus_id.as_ref()),
        )
        .then_with(|| {
            js_locale_compare(
                &js_string(left.title.as_ref()),
                &js_string(right.title.as_ref()),
            )
        })
    });
    majors
}

fn get_alias_title_parts(plan: &Value) -> Option<(String, String)> {
    for field in ["title", "shortTitle"] {
        let title = js_string_nullish(plan.get(field));
        if let Some(parts) = split_alias_title(&title) {
            return Some(parts);
        }
    }
    None
}

fn split_alias_title(title: &str) -> Option<(String, String)> {
    let colon_index = title.find(':')?;
    let parent_title = &title[..colon_index];
    if parent_title.is_empty() || contains_js_line_terminator(parent_title) {
        return None;
    }
    let mut option_start = colon_index + 1;
    while let Some(character) = title[option_start..].chars().next() {
        if !is_js_whitespace(character) {
            break;
        }
        option_start += character.len_utf8();
    }
    let option = &title[option_start..];
    if option.is_empty()
        || option
            .chars()
            .next()
            .is_some_and(|character| matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        return None;
    }

    let mut option_end = option.len();
    if option.ends_with(')') {
        let mut found_suffix = None;
        for (open_index, character) in option.char_indices() {
            if character != '(' {
                continue;
            }
            let whitespace_start = option[..open_index]
                .char_indices()
                .rev()
                .take_while(|(_, previous)| is_js_whitespace(*previous))
                .last()
                .map(|(index, _)| index);
            let Some(whitespace_start) = whitespace_start else {
                continue;
            };
            let parent_option = &option[..whitespace_start];
            if parent_option.is_empty() || contains_js_line_terminator(parent_option) {
                continue;
            }
            let inner = &option[open_index + 1..option.len() - 1];
            if inner.is_empty() || inner.contains(')') {
                continue;
            }
            found_suffix = Some(whitespace_start);
            break;
        }
        if let Some(end) = found_suffix {
            option_end = end;
        }
    }
    let option_title = option[..option_end].to_string();
    if option_title.is_empty() || contains_js_line_terminator(&option_title) {
        return None;
    }
    Some((parent_title.to_string(), option_title))
}

fn normalize_alias_text(value: Option<&Value>) -> String {
    let mut text = js_string_nullish(value).replace('&', " and ");
    let chars = text.chars().collect::<Vec<_>>();
    let mut without_parenthetical = String::with_capacity(text.len());
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '(' {
            if let Some(relative_close) = chars[index + 1..]
                .iter()
                .position(|character| *character == ')')
            {
                without_parenthetical.push(' ');
                index += relative_close + 2;
                continue;
            }
        }
        without_parenthetical.push(chars[index]);
        index += 1;
    }
    text = without_parenthetical.to_lowercase();

    let mut normalized = String::new();
    let mut pending_hyphen = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_hyphen && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(character);
            pending_hyphen = false;
        } else {
            pending_hyphen = true;
        }
    }
    normalized
}

fn direct_commands(plan_id: &str) -> [String; 4] {
    let quoted = shell_quote(plan_id);
    [
        format!(
            "node scripts/planner/parse-transfer-planner-requirement-sources.cjs --target-plan-id {quoted}"
        ),
        format!(
            "node scripts/planner/assert-transfer-planner-owner-source.cjs --target-plan-id {quoted} --all-owners"
        ),
        format!(
            "node scripts/planner/probe-transfer-planner-owner-source-links.cjs --target-plan-id {quoted}"
        ),
        format!(
            "node scripts/planner/audit-transfer-planner-source-backed-coverage.cjs --target-plan-id {quoted} --report-only"
        ),
    ]
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .any(|character| is_js_whitespace(character) || character == '"' || character == '\\')
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn print_major_list(majors: &[Major]) {
    println!("Planner majors: {}", majors.len());
    for major in majors {
        println!(
            "{}\t{}\t{}\t{}",
            js_string(major.plan_id.as_ref()),
            js_string(major.campus_id.as_ref()),
            major.pathway_count,
            js_string(major.title.as_ref())
        );
    }
}

fn build_command_markdown(majors: &[Major]) -> String {
    let mut lines = vec![
        "# Planner Major Spot-Check Commands".to_string(),
        String::new(),
        "Run these from `source/`. Each block is targeted to one major and avoids full planner verification.".to_string(),
        String::new(),
    ];
    for major in majors {
        lines.push(format!("## {}", js_string(major.title.as_ref())));
        lines.push(String::new());
        lines.push(format!("Plan id: `{}`", js_string(major.plan_id.as_ref())));
        lines.push(String::new());
        lines.push("```powershell".to_string());
        lines.extend(direct_commands(&js_string(major.plan_id.as_ref())));
        lines.push("```".to_string());
        lines.push(String::new());
    }
    let joined = lines.join("\n");
    format!("{}\n", joined.trim_end_matches(is_js_whitespace))
}

fn print_commands(majors: &[Major], write_path: Option<&Path>) -> Result<(), String> {
    let markdown = build_command_markdown(majors);
    if let Some(write_path) = write_path {
        write_markdown_report(write_path, &markdown)?;
        println!("Wrote {}", write_path.display());
    } else {
        print!("{markdown}");
    }
    Ok(())
}

fn run_selected_checks(
    source_root: &Path,
    plan_id: &str,
    options: &SpotCheckOptions,
) -> Result<bool, String> {
    let phases = [
        (
            options.refresh,
            vec![
                "scripts/planner/refresh-transfer-planner-sources.cjs",
                "--target-plan-id",
                plan_id,
            ],
        ),
        (
            options.parse,
            vec![
                "scripts/planner/parse-transfer-planner-requirement-sources.cjs",
                "--target-plan-id",
                plan_id,
            ],
        ),
        (
            options.assert,
            vec![
                "scripts/planner/assert-transfer-planner-owner-source.cjs",
                "--target-plan-id",
                plan_id,
                "--all-owners",
            ],
        ),
        (
            options.probe,
            vec![
                "scripts/planner/probe-transfer-planner-owner-source-links.cjs",
                "--target-plan-id",
                plan_id,
            ],
        ),
    ];
    for (enabled, phase_args) in phases {
        if enabled {
            run_node(source_root, &phase_args)?;
        }
    }
    Ok(true)
}

fn run_node(source_root: &Path, args: &[&str]) -> Result<(), String> {
    let display = args
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ");
    println!("\n> node {display}");
    let error_message = format!("Spot-check command failed: node {}", args.join(" "));
    let status = Command::new("node")
        .args(args)
        .current_dir(source_root)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|_| error_message.clone())?;
    if !status.success() {
        return Err(error_message);
    }
    Ok(())
}

fn parse_options(args: &[String]) -> SpotCheckOptions {
    let value_flags = [
        "--target-plan-id",
        "--plan-id",
        "--target",
        "--campus-id",
        "--write-command-file",
    ];
    let positional = get_positional_args(args, &value_flags);
    let positional_campus = positional
        .iter()
        .find(|argument| VALID_CAMPUSES.contains(&argument.as_str()))
        .cloned();
    let list = has_arg(args, &["--list"]);
    let commands = has_arg(args, &["--commands"]);
    let write_command_file = has_arg(args, &["--write-command-file"]);
    let campus_id = get_arg_value(args, &["--campus-id"]).or(positional_campus);
    let plan_id =
        get_arg_value(args, &["--target-plan-id", "--plan-id", "--target"]).or_else(|| {
            (!(list || commands))
                .then(|| positional.first().cloned())
                .flatten()
        });
    let has_positional = |value: &str| positional.iter().any(|argument| argument == value);
    SpotCheckOptions {
        campus_id,
        plan_id,
        list,
        commands,
        write_command_file,
        write_path: get_arg_value(args, &["--write-command-file"]),
        refresh: has_arg(args, &["--refresh"]) || has_positional("refresh"),
        parse: has_arg(args, &["--parse", "--standard"])
            || has_positional("parse")
            || has_positional("standard"),
        assert: has_arg(args, &["--assert", "--standard"])
            || has_positional("assert")
            || has_positional("standard"),
        probe: has_arg(args, &["--probe"]) || has_positional("probe"),
    }
}

fn has_arg(args: &[String], flags: &[&str]) -> bool {
    args.iter()
        .any(|argument| flags.contains(&argument.as_str()))
}

fn get_arg_value(args: &[String], flags: &[&str]) -> Option<String> {
    for flag in flags {
        let direct_prefix = format!("{flag}=");
        if let Some(argument) = args
            .iter()
            .find(|argument| argument.starts_with(&direct_prefix))
        {
            let value = trim_js_whitespace(&argument[direct_prefix.len()..]);
            return (!value.is_empty()).then(|| value.to_string());
        }
        if let Some(index) = args.iter().position(|argument| argument == flag) {
            let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) else {
                return None;
            };
            let value = trim_js_whitespace(value);
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

fn get_positional_args(args: &[String], value_flags: &[&str]) -> Vec<String> {
    let mut positional = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let argument = &args[index];
        if argument.starts_with("--") {
            let (flag, has_equals) = argument
                .find('=')
                .map(|equals| (&argument[..equals], true))
                .unwrap_or((argument.as_str(), false));
            if !has_equals && value_flags.contains(&flag) {
                index += 1;
            }
        } else {
            positional.push(argument.clone());
        }
        index += 1;
    }
    positional
}

fn resolve_path(source_root: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        source_root.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string(Some(value))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

fn js_string_nullish(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(value) => js_string(Some(value)),
    }
}

fn js_locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    compare_locale_folded(&left.to_lowercase(), &right.to_lowercase()).then_with(|| right.cmp(left))
}

fn compare_locale_folded(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_chars = left.chars();
    let mut right_chars = right.chars();
    loop {
        match (left_chars.next(), right_chars.next()) {
            (Some(left), Some(right)) => {
                let ordering = locale_char_key(left).cmp(&locale_char_key(right));
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, None) => return std::cmp::Ordering::Equal,
        }
    }
}

fn locale_char_key(character: char) -> (u8, u32) {
    const LOCALE_PUNCTUATION: &str = " _-–,;:!?.'\\\"()[]{}@*/&#%`^+<=>|~$";
    if let Some(index) = LOCALE_PUNCTUATION
        .chars()
        .position(|item| item == character)
    {
        return (0, index as u32);
    }
    if character.is_ascii_digit() {
        return (1, character as u32);
    }
    if character.is_ascii_alphabetic() {
        return (2, character as u32);
    }
    (3, character as u32)
}

fn trim_js_whitespace(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

fn is_js_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

fn contains_js_line_terminator(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
}

fn print_usage() {
    println!(
        "Usage:\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --list\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --commands [--write-command-file]\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --target-plan-id <plan-id> [--parse] [--assert] [--probe] [--refresh]\n\nOptions:\n  --target-plan-id <plan-id>   Select one major to spot check.\n  --campus-id <campus-id>      Filter --list or --commands output.\n  --list                       List every spot-checkable major.\n  --commands                   Print direct node commands for every major.\n  --write-command-file [path]  Write the command list to .tmp/reports/planner-major-spot-check-commands.md.\n  --parse                      Run targeted requirement parsing for the selected major.\n  --assert                     Assert the selected major exists in the current parse report.\n  --probe                      Probe source links for the selected major.\n  --refresh                    Run targeted source refresh for the selected major.\n  --standard                   Run the normal low-risk spot check: --parse then --assert.\n\nExamples:\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --list\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --commands --write-command-file\n  cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major --target-plan-id uw-bothell-electrical-engineering --standard"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(id: &str, campus: &str, title: &str, pathways: usize) -> Value {
        serde_json::json!({
            "id": id,
            "campusId": campus,
            "title": title,
            "pathways": vec![Value::Null; pathways],
        })
    }

    #[test]
    fn generated_typescript_array_scanner_handles_nested_values_and_escaped_brackets() {
        let source = r#"
export const TRANSFER_PLANNER_BOOTSTRAP_ALL_MAJOR_PLANS: unknown[] = [
  {"id":"plan]x","campusId":"uw-seattle","title":"Business: Accounting","pathways":[{"text":"escaped \"]\" bracket"}]}
] as const;
"#;
        let parsed = parse_top_level_json_array(source, BOOTSTRAP_EXPORT, "fixture.ts").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["id"], "plan]x");
        assert_eq!(parsed[0]["pathways"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn alias_title_parser_preserves_parenthetical_and_line_break_semantics() {
        assert_eq!(
            split_alias_title("Business & Management: Accounting (B.A.)"),
            Some((
                "Business & Management".to_string(),
                "Accounting".to_string()
            ))
        );
        assert_eq!(
            split_alias_title("Business: Accounting\n (MRP)"),
            Some(("Business".to_string(), "Accounting".to_string()))
        );
        assert_eq!(
            split_alias_title("Business: Accounting\n and Finance"),
            None
        );
    }

    #[test]
    fn locale_sort_matches_default_node_punctuation_order_for_plan_titles() {
        assert_eq!(
            js_locale_compare("Criminal Justice - Online", "Criminal Justice (BA)"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            js_locale_compare("uw-bothell", "uw-seattle"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn planner_major_selection_hides_alias_children_only_when_parent_exists_on_same_campus() {
        let plans = vec![
            plan("parent", "uw-seattle", "Business & Management", 2),
            plan(
                "child",
                "uw-seattle",
                "Business and Management: Accounting",
                1,
            ),
            plan(
                "other-campus",
                "uw-tacoma",
                "Business and Management: Accounting",
                3,
            ),
            plan("unrelated", "uw-bothell", "Computer Science", 0),
        ];
        let majors = planner_majors(&plans, None);
        assert_eq!(
            majors
                .iter()
                .map(|major| js_string(major.plan_id.as_ref()))
                .collect::<Vec<_>>(),
            ["unrelated", "parent", "other-campus"]
        );
        let seattle = planner_majors(&plans, Some("uw-seattle"));
        assert_eq!(seattle.len(), 1);
        assert_eq!(js_string(seattle[0].plan_id.as_ref()), "parent");
    }

    #[test]
    fn command_list_and_shell_quoting_match_the_legacy_text_contract() {
        assert_eq!(shell_quote("uw-seattle-plan"), "uw-seattle-plan");
        assert_eq!(shell_quote("plan id"), "\"plan id\"");
        assert_eq!(shell_quote("plan\\id"), "\"plan\\\\id\"");
        let majors = vec![Major {
            plan_id: Some(Value::String("plan id".to_string())),
            campus_id: Some(Value::String("uw-seattle".to_string())),
            title: Some(Value::String("Computer Science".to_string())),
            pathway_count: 2,
        }];
        let markdown = build_command_markdown(&majors);
        assert!(markdown.starts_with("# Planner Major Spot-Check Commands\n"));
        assert!(markdown.contains("Plan id: `plan id`"));
        assert!(markdown.contains("--target-plan-id \"plan id\""));
        assert!(markdown.ends_with("\n"));
    }

    #[test]
    fn argument_parser_preserves_alias_and_positional_phase_behavior() {
        let options = parse_options(&[
            "uw-seattle".to_string(),
            "--target".to_string(),
            "plan-id".to_string(),
            "standard".to_string(),
            "probe".to_string(),
        ]);
        assert_eq!(options.campus_id.as_deref(), Some("uw-seattle"));
        assert_eq!(options.plan_id.as_deref(), Some("plan-id"));
        assert_eq!(
            get_arg_value(
                &[
                    "--target".to_string(),
                    "\u{FEFF}plan-id\u{FEFF}".to_string()
                ],
                &["--target"]
            )
            .as_deref(),
            Some("plan-id")
        );
        assert!(options.parse && options.assert && options.probe);
        assert!(!options.refresh);
    }
}
