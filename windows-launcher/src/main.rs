use reqwest::blocking::Client;
use serde_json::Value;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const REPOSITORY_URL: &str = "https://github.com/MarsLuay/GatorGuide.git";
const REPOSITORY_DIRECTORY: &str = "GatorGuide";
const BACK_EXIT_CODE: i32 = 86;
const BACK_EXIT_CODE_TEXT: &str = "86";
const EXPO_PORT: u16 = 8081;
const EXPO_URL: &str = "http://127.0.0.1:8081";

type LauncherResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LauncherKind {
    Startup,
    CoursePlanner,
    Resources,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupMode {
    Normal,
    Demo,
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("GatorGuide launcher failed: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> LauncherResult<i32> {
    let mut args = env::args_os();
    let executable = args.next().unwrap_or_default();
    let kind = launcher_kind(&executable);
    let user_args = args.collect::<Vec<_>>();

    if user_args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help(kind);
        return Ok(0);
    }

    let startup_mode = if kind == LauncherKind::Startup {
        Some(choose_startup_mode()?)
    } else {
        None
    };
    let executable_directory = env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(env::current_dir()?);
    let repo_root = locate_or_clone_repository(&executable_directory)?;

    match kind {
        LauncherKind::Startup => run_startup_launcher(
            &repo_root,
            startup_mode.expect("startup mode is selected for the startup launcher"),
        ),
        LauncherKind::CoursePlanner => run_course_planner_launcher(&repo_root, &user_args),
        LauncherKind::Resources => run_resource_launcher(&repo_root, &user_args),
    }
}

fn launcher_kind(executable: &OsStr) -> LauncherKind {
    let name = Path::new(executable)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("start-to-run") {
        LauncherKind::Startup
    } else if name.contains("add-or-remove-resources") {
        LauncherKind::Resources
    } else {
        LauncherKind::CoursePlanner
    }
}

fn print_help(kind: LauncherKind) {
    match kind {
        LauncherKind::Startup => println!(
            "GatorGuide startup launcher\n\nUsage: Start-to-run.exe [--help]\n\nChoose normal or demo mode, prepare the local app dependencies, and start Expo."
        ),
        LauncherKind::Resources => println!(
            "GatorGuide resource launcher\n\nUsage: add-or-remove-resources.exe [arguments]\n\nPasses arguments to source/scripts/assets/add-catalog-item.cjs."
        ),
        LauncherKind::CoursePlanner => println!(
            "GatorGuide course planner updater\n\nUsage:\n  Course-Planner-Updater.exe\n  Course-Planner-Updater.exe maintenance\n  Course-Planner-Updater.exe maintenance-no-downloads\n  Course-Planner-Updater.exe refresh\n  Course-Planner-Updater.exe refresh-no-downloads\n  Course-Planner-Updater.exe cache-summary\n  Course-Planner-Updater.exe edit-course-links\n  Course-Planner-Updater.exe laymans-diagnosis\n  Course-Planner-Updater.exe export-fact-check"
        ),
    }
}

fn locate_or_clone_repository(launcher_directory: &Path) -> LauncherResult<PathBuf> {
    if let Some(root) = find_repository_root(launcher_directory) {
        return Ok(root);
    }

    let clone_root = launcher_directory.join(REPOSITORY_DIRECTORY);
    if let Some(root) = find_repository_root(&clone_root) {
        println!("Found Gator Guide in {}.", root.display());
        return Ok(root);
    }
    if clone_root.exists() {
        return Err(format!(
            "Cannot clone into {} because that path already exists but is not a GatorGuide checkout.",
            clone_root.display()
        )
        .into());
    }

    let git = ensure_git()?;
    println!("Gator Guide was not found next to this launcher. Cloning {REPOSITORY_URL}...");
    let status = Command::new(git)
        .arg("clone")
        .arg(REPOSITORY_URL)
        .arg(&clone_root)
        .status()?;
    if !status.success() {
        return Err(format!("git clone failed with exit code {}", status_code(status)).into());
    }
    find_repository_root(&clone_root).ok_or_else(|| {
        format!(
            "The repository finished cloning, but {} does not contain source/package.json.",
            clone_root.display()
        )
        .into()
    })
}

fn find_repository_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("source").join("package.json").is_file() {
            return Some(current);
        }
        if current.join("source").join("scripts").is_dir()
            && current.join("automation").join("Cargo.toml").is_file()
        {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn ensure_git() -> LauncherResult<PathBuf> {
    if let Some(path) = find_tool("git") {
        return Ok(path);
    }
    println!("Git was not found. Trying to install it automatically...");
    try_package_managers("Git.Git", "git", "git")?;
    if let Some(path) = find_tool("git") {
        println!("Git finished installing successfully.");
        return Ok(path);
    }

    println!("Package-manager installation did not expose Git. Trying the official installer...");
    install_git_direct()?;
    find_tool("git").ok_or_else(|| {
        "Git was installed, but this terminal cannot find it yet. Close the window and run the launcher again.".into()
    })
}

fn ensure_node_toolchain() -> LauncherResult<PathBuf> {
    if let Some(path) = find_tool("node")
        && find_tool("npm").is_some()
        && find_tool("npx").is_some()
    {
        return Ok(path);
    }
    println!("Node.js, npm, or npx was not found. Trying to install Node.js LTS...");
    try_package_managers("OpenJS.NodeJS.LTS", "nodejs-lts", "nodejs-lts")?;
    if let Some(path) = find_tool("node")
        && find_tool("npm").is_some()
        && find_tool("npx").is_some()
    {
        println!("Node.js finished installing successfully.");
        return Ok(path);
    }

    println!(
        "Package-manager installation did not expose Node.js. Trying the official installer..."
    );
    install_node_direct()?;
    let node = find_tool("node").ok_or_else(|| {
        "Node.js was installed, but this terminal cannot find it yet. Close the window and run the launcher again.".to_string()
    })?;
    if find_tool("npm").is_none() || find_tool("npx").is_none() {
        return Err("npm or npx is still unavailable after installing Node.js. Close the window and run the launcher again.".into());
    }
    Ok(node)
}

fn try_package_managers(
    winget_id: &str,
    chocolatey_package: &str,
    scoop_package: &str,
) -> LauncherResult<()> {
    let installers = [
        (
            "winget",
            vec![
                "install",
                "--id",
                winget_id,
                "--exact",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--silent",
            ],
        ),
        (
            "choco",
            vec!["install", chocolatey_package, "-y", "--no-progress"],
        ),
        ("scoop", vec!["install", scoop_package]),
    ];
    for (program, args) in installers {
        let Some(executable) = find_tool(program) else {
            continue;
        };
        println!("Trying {program}...");
        let status = Command::new(executable).args(args).status()?;
        if status.success() {
            return Ok(());
        }
        eprintln!(
            "{program} installation failed (exit {}).",
            status_code(status)
        );
    }
    Ok(())
}

fn install_git_direct() -> LauncherResult<()> {
    let client = http_client()?;
    let release: Value = client
        .get("https://api.github.com/repos/git-for-windows/git/releases/latest")
        .send()?
        .error_for_status()?
        .json()?;
    let installer_url = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find_map(|asset| {
                let name = asset["name"].as_str()?;
                let url = asset["browser_download_url"].as_str()?;
                (name.ends_with("64-bit.exe") && !name.contains("Portable")).then_some(url)
            })
        })
        .ok_or("Could not find the 64-bit Git for Windows installer in the latest release")?;
    let installer = env::temp_dir().join("GatorGuide-Git-64-bit-installer.exe");
    download_file(&client, installer_url, &installer)?;
    println!("Installing Git from the official Git for Windows release...");
    let status = Command::new(&installer)
        .args(["/VERYSILENT", "/NORESTART", "/NOCANCEL", "/SP-"])
        .status()?;
    let _ = fs::remove_file(installer);
    if !status.success() {
        return Err(format!(
            "Git installer failed with exit code {}",
            status_code(status)
        )
        .into());
    }
    Ok(())
}

fn install_node_direct() -> LauncherResult<()> {
    let client = http_client()?;
    let releases: Vec<Value> = client
        .get("https://nodejs.org/dist/index.json")
        .send()?
        .error_for_status()?
        .json()?;
    let version = releases
        .iter()
        .find(|release| release["lts"].as_bool() == Some(true))
        .and_then(|release| release["version"].as_str())
        .ok_or("Could not resolve the latest Node.js LTS version")?;
    let filename = format!("node-{version}-x64.msi");
    let url = format!("https://nodejs.org/dist/{version}/{filename}");
    let installer = env::temp_dir().join(filename);
    download_file(&client, &url, &installer)?;
    println!("Installing Node.js {version} from nodejs.org...");
    let msiexec = find_tool("msiexec").unwrap_or_else(|| PathBuf::from("msiexec.exe"));
    let status = Command::new(msiexec)
        .args(["/i"])
        .arg(&installer)
        .args(["/quiet", "/norestart"])
        .status()?;
    let _ = fs::remove_file(installer);
    if !status.success() && status.code() != Some(3010) {
        return Err(format!(
            "Node.js installer failed with exit code {}",
            status_code(status)
        )
        .into());
    }
    Ok(())
}

fn http_client() -> LauncherResult<Client> {
    Ok(Client::builder()
        .user_agent("GatorGuide-Rust-Launcher/1.0")
        .build()?)
}

fn download_file(client: &Client, url: &str, destination: &Path) -> LauncherResult<()> {
    let bytes = client.get(url).send()?.error_for_status()?.bytes()?;
    fs::write(destination, &bytes)?;
    Ok(())
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let lowercase_name = name.to_ascii_lowercase();
    let extensions = if cfg!(windows) {
        match lowercase_name.as_str() {
            "npm" => vec!["npm.cmd", "npm.exe"],
            "npx" => vec!["npx.cmd", "npx.exe"],
            "powershell" => vec!["powershell.exe"],
            "git" => vec!["git.exe"],
            "node" => vec!["node.exe"],
            "winget" => vec!["winget.exe"],
            "choco" => vec!["choco.exe"],
            "scoop" => vec!["scoop.exe"],
            "msiexec" => vec!["msiexec.exe"],
            "cmd" => vec!["cmd.exe"],
            other => vec![other],
        }
    } else {
        vec![name]
    };

    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            for extension in &extensions {
                candidates.push(directory.join(extension));
            }
        }
    }
    candidates.extend(known_tool_paths(name));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn known_tool_paths(name: &str) -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut roots = Vec::new();
    for variable in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LocalAppData",
        "USERPROFILE",
    ] {
        if let Some(value) = env::var_os(variable) {
            roots.push(PathBuf::from(value));
        }
    }
    let mut paths = Vec::new();
    match name.to_ascii_lowercase().as_str() {
        "git" => {
            for root in &roots {
                paths.extend([
                    root.join("Git/cmd/git.exe"),
                    root.join("Git/bin/git.exe"),
                    root.join("Programs/Git/cmd/git.exe"),
                    root.join("Programs/Git/bin/git.exe"),
                    root.join("scoop/apps/git/current/cmd/git.exe"),
                ]);
            }
        }
        "node" => {
            for root in &roots {
                paths.extend([
                    root.join("nodejs/node.exe"),
                    root.join("Programs/nodejs/node.exe"),
                    root.join("scoop/apps/nodejs-lts/current/node.exe"),
                ]);
            }
        }
        "npm" | "npx" => {
            let executable = if name.eq_ignore_ascii_case("npm") {
                "npm.cmd"
            } else {
                "npx.cmd"
            };
            for root in &roots {
                paths.extend([
                    root.join(format!("nodejs/{executable}")),
                    root.join(format!("Programs/nodejs/{executable}")),
                    root.join(format!("scoop/apps/nodejs-lts/current/{executable}")),
                ]);
            }
        }
        "powershell" => {
            if let Some(windows) = env::var_os("SystemRoot") {
                let root = PathBuf::from(windows);
                paths.extend([
                    root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
                    root.join("SysWOW64/WindowsPowerShell/v1.0/powershell.exe"),
                ]);
            }
        }
        "winget" => {
            if let Some(root) = env::var_os("LocalAppData") {
                paths.push(PathBuf::from(root).join("Microsoft/WindowsApps/winget.exe"));
            }
        }
        "cmd" | "msiexec" => {
            if let Some(root) = env::var_os("SystemRoot") {
                let executable = if name.eq_ignore_ascii_case("cmd") {
                    "cmd.exe"
                } else {
                    "msiexec.exe"
                };
                paths.push(PathBuf::from(root).join(format!("System32/{executable}")));
            }
        }
        _ => {}
    }
    paths
}

fn run_startup_launcher(repo_root: &Path, mode: StartupMode) -> LauncherResult<i32> {
    let app_root = repo_root.join("source");
    ensure_node_toolchain()?;
    prepare_env_file(&app_root)?;
    ensure_app_dependencies(&app_root)?;
    sync_expo_packages(&app_root)?;
    println!("Starting Expo...");
    println!("The Expo page will open in your default browser when it is ready.");
    let mut child = spawn_npm(&app_root, &["run", "start"], startup_environment(mode))?;
    open_browser_when_ready(&mut child)?;
    let status = child.wait()?;
    organize_tmp_artifacts(&app_root);
    if status.success() {
        Ok(0)
    } else {
        eprintln!("Expo exited with code {}.", status_code(status));
        Ok(1)
    }
}

fn choose_startup_mode() -> LauncherResult<StartupMode> {
    if let Some(mode) = env::var_os("GATORGUIDE_STARTUP_MODE")
        && let Some(mode) = normalize_startup_mode(&mode.to_string_lossy())
    {
        return Ok(mode);
    }
    loop {
        println!("\nChoose startup mode:\n  1. Run normally\n  2. Demo mode\n");
        print!("Enter 1 or 2: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        match choice.trim() {
            "1" => return Ok(StartupMode::Normal),
            "2" => return Ok(StartupMode::Demo),
            _ => println!("Please enter 1 or 2."),
        }
    }
}

fn normalize_startup_mode(value: &str) -> Option<StartupMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "run" | "normal" => Some(StartupMode::Normal),
        "2" | "demo" => Some(StartupMode::Demo),
        _ => None,
    }
}

fn startup_environment(mode: StartupMode) -> Vec<(&'static str, &'static str)> {
    match mode {
        StartupMode::Normal => vec![
            ("GATORGUIDE_STARTUP_MODE", "normal"),
            ("GATORGUIDE_DEMO_MODE", ""),
            ("EXPO_PUBLIC_GATORGUIDE_DEMO_MODE", ""),
        ],
        StartupMode::Demo => vec![
            ("GATORGUIDE_STARTUP_MODE", "demo"),
            ("GATORGUIDE_DEMO_MODE", "1"),
            ("EXPO_PUBLIC_GATORGUIDE_DEMO_MODE", "1"),
        ],
    }
}

fn prepare_env_file(app_root: &Path) -> LauncherResult<()> {
    let env_file = app_root.join(".env");
    let example = app_root.join(".env.example");
    if env_file.exists() {
        println!("Existing .env found. Skipping env setup.");
    } else if example.is_file() {
        fs::copy(&example, &env_file)?;
        println!("Created source/.env from .env.example.");
    }
    Ok(())
}

fn ensure_app_dependencies(app_root: &Path) -> LauncherResult<()> {
    let node_modules = app_root.join("node_modules");
    if node_modules.exists() {
        println!("Checking app dependencies...");
        if run_npm(app_root, &["ls", "--depth=0"], &[], true)? == 0 {
            println!("App dependencies are already installed.");
            return Ok(());
        }
        println!("App dependencies are incomplete. Reinstalling...");
    }

    println!("Installing app dependencies. This may take a few minutes...");
    let mut code = run_npm(app_root, &["ci"], &[], false)?;
    if code != 0 {
        println!("npm ci failed, trying npm install instead...");
        code = run_npm(app_root, &["install"], &[], false)?;
    }
    if code != 0 {
        return Err(format!("Installing app dependencies failed with exit code {code}.").into());
    }
    println!("App dependencies installed successfully.");
    Ok(())
}

fn sync_expo_packages(app_root: &Path) -> LauncherResult<()> {
    println!("Checking Expo package compatibility...");
    if run_npx(app_root, &["expo", "install", "--check"], true)? == 0 {
        println!("Expo packages are already compatible with this SDK.");
        return Ok(());
    }

    println!("Updating Expo-managed packages for this SDK...");
    let code = run_npx(app_root, &["expo", "install", "--fix", "--npm"], false)?;
    if code != 0 {
        return Err(format!("Expo package update failed with exit code {code}.").into());
    }
    println!("Expo packages updated successfully.");
    Ok(())
}

fn open_browser_when_ready(child: &mut Child) -> LauncherResult<()> {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if TcpStream::connect(("127.0.0.1", EXPO_PORT)).is_ok() {
            #[cfg(windows)]
            {
                let _ = Command::new("explorer.exe").arg(EXPO_URL).spawn();
            }
            #[cfg(not(windows))]
            {
                let opener = if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                };
                let _ = Command::new(opener).arg(EXPO_URL).spawn();
            }
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!(
                "Expo exited before port {EXPO_PORT} became available (exit {}).",
                status_code(status)
            )
            .into());
        }
        if Instant::now() >= deadline {
            println!(
                "Expo did not open port {EXPO_PORT} within two minutes; continuing to wait for it."
            );
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn spawn_npm(
    working_directory: &Path,
    arguments: &[&str],
    environment: Vec<(&str, &str)>,
) -> LauncherResult<Child> {
    let command = npm_command_line(arguments);
    let mut process = shell_command(&command);
    process.current_dir(working_directory);
    for (key, value) in environment {
        process.env(key, value);
    }
    Ok(process.spawn()?)
}

fn run_npm(
    working_directory: &Path,
    arguments: &[&str],
    environment: &[(&str, &str)],
    quiet: bool,
) -> LauncherResult<i32> {
    let command = npm_command_line(arguments);
    let mut process = shell_command(&command);
    process.current_dir(working_directory);
    for (key, value) in environment {
        process.env(key, value);
    }
    if quiet {
        process.stdout(Stdio::null()).stderr(Stdio::null());
    }
    Ok(status_code(process.status()?))
}

fn run_npx(working_directory: &Path, arguments: &[&str], quiet: bool) -> LauncherResult<i32> {
    let command = format!("npx {}", arguments.join(" "));
    let mut process = shell_command(&command);
    process.current_dir(working_directory);
    if quiet {
        process.stdout(Stdio::null()).stderr(Stdio::null());
    }
    Ok(status_code(process.status()?))
}

fn npm_command_line(arguments: &[&str]) -> String {
    format!("npm {}", arguments.join(" "))
}

fn path_with_launcher_tools() -> Option<OsString> {
    let mut paths = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    for tool in ["node", "npm", "npx", "git", "cmd"] {
        if let Some(directory) =
            find_tool(tool).and_then(|path| path.parent().map(Path::to_path_buf))
            && !paths.contains(&directory)
        {
            paths.push(directory);
        }
    }
    env::join_paths(paths).ok()
}

fn shell_command(command_line: &str) -> Command {
    if cfg!(windows) {
        let executable = find_tool("cmd").unwrap_or_else(|| PathBuf::from("cmd.exe"));
        let mut command = Command::new(executable);
        command.args(["/D", "/S", "/C", command_line]);
        if let Some(path) = path_with_launcher_tools() {
            command.env("PATH", path);
        }
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-lc", command_line]);
        command
    }
}

fn run_course_planner_launcher(repo_root: &Path, args: &[OsString]) -> LauncherResult<i32> {
    if args.is_empty() {
        run_course_planner_menu(repo_root)
    } else {
        let mode = args[0].to_string_lossy().to_ascii_lowercase();
        let result = run_planner_mode(repo_root, &mode, false)?;
        if !matches!(mode.as_str(), "help" | "-h" | "--help") {
            finish_planner_action(repo_root, "Course planner", result, false)?;
        }
        Ok(result)
    }
}

fn run_course_planner_menu(repo_root: &Path) -> LauncherResult<i32> {
    loop {
        println!(
            "\nCourse Planner Updater\n1. Course updates + tests\n2. Course updates only\n3. Show cache summary\n4. Edit course links\n5. Laymans Diagnosis\n6. Export course planner row document\n7. Back\n"
        );
        let choice = prompt("Enter 1-7: ")?;
        match choice.trim() {
            "1" => run_maintenance_submenu(repo_root)?,
            "2" => run_refresh_submenu(repo_root)?,
            "3" => run_menu_action(repo_root, "cache-summary", "Course planner cache summary")?,
            "4" => {
                let result = run_planner_mode(repo_root, "edit-course-links", true)?;
                if result != BACK_EXIT_CODE {
                    finish_planner_action(repo_root, "Edit course links", result, true)?;
                }
            }
            "5" => run_menu_action(repo_root, "laymans-diagnosis", "Laymans Diagnosis")?,
            "6" => run_menu_action(
                repo_root,
                "export-fact-check",
                "Course planner row document export",
            )?,
            "7" => return Ok(0),
            _ => println!("Enter a number from 1 to 7."),
        }
    }
}

fn run_maintenance_submenu(repo_root: &Path) -> LauncherResult<()> {
    loop {
        println!("\nCourse updates + tests\n1. Normal\n2. Skip downloads\nB. Back\n");
        let choice = prompt("Enter 1-2 or B: ")?;
        let mode = match choice.trim().to_ascii_lowercase().as_str() {
            "1" => "maintenance",
            "2" => "maintenance-no-downloads",
            "b" => return Ok(()),
            _ => {
                println!("Enter 1, 2, or B.");
                continue;
            }
        };
        let result = run_planner_mode(repo_root, mode, true)?;
        if result == BACK_EXIT_CODE {
            continue;
        }
        finish_planner_action(repo_root, "Course planner maintenance", result, true)?;
        return Ok(());
    }
}

fn run_refresh_submenu(repo_root: &Path) -> LauncherResult<()> {
    loop {
        println!("\nCourse updates only\n1. Normal\n2. Skip downloads\nB. Back\n");
        let choice = prompt("Enter 1-2 or B: ")?;
        let mode = match choice.trim().to_ascii_lowercase().as_str() {
            "1" => "refresh",
            "2" => "refresh-no-downloads",
            "b" => return Ok(()),
            _ => {
                println!("Enter 1, 2, or B.");
                continue;
            }
        };
        let result = run_planner_mode(repo_root, mode, false)?;
        finish_planner_action(repo_root, "Course planner refresh", result, true)?;
        return Ok(());
    }
}

fn run_menu_action(repo_root: &Path, mode: &str, label: &str) -> LauncherResult<()> {
    let result = run_planner_mode(repo_root, mode, false)?;
    finish_planner_action(repo_root, label, result, true)
}

fn run_planner_mode(repo_root: &Path, mode: &str, allow_back: bool) -> LauncherResult<i32> {
    let app_root = repo_root.join("source");
    let scripts = app_root.join("scripts");
    let maintenance_script = scripts.join("run-transfer-planner-maintenance.ps1");
    let refresh_script = scripts.join("run-transfer-planner-refresh.ps1");
    let back_args = if allow_back {
        vec!["-BackExitCode", BACK_EXIT_CODE_TEXT]
    } else {
        Vec::new()
    };
    match mode {
        "maintenance" | "maintenance-no-downloads" => {
            let mut args = vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"];
            args.push(path_as_str(&maintenance_script)?);
            if mode == "maintenance-no-downloads" {
                args.push("-SkipDownloads");
            }
            args.push("-NoPrompt");
            args.push("-RunPostChecks");
            args.extend(back_args.iter().copied());
            let code = run_powershell(&app_root, &args)?;
            if code == 0 || (allow_back && code == BACK_EXIT_CODE) {
                return Ok(code);
            }
            eprintln!(
                "PowerShell failed (exit {code}). Falling back to Node maintenance runner..."
            );
            run_node_maintenance(
                &app_root,
                if mode == "maintenance-no-downloads" {
                    &["--skip-downloads"]
                } else {
                    &[]
                },
            )
        }
        "refresh" | "refresh-no-downloads" => {
            let mut args = vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"];
            args.push(path_as_str(&refresh_script)?);
            if mode == "refresh-no-downloads" {
                args.push("-SkipDownloads");
            }
            args.push("-SkipVerify");
            let code = run_powershell(&app_root, &args)?;
            if code == 0 {
                return Ok(0);
            }
            eprintln!("PowerShell failed (exit {code}). Falling back to Node refresh runner...");
            let mut node_args = Vec::new();
            if mode == "refresh-no-downloads" {
                node_args.push("--skip-downloads");
            }
            node_args.extend(["--refresh-only", "--skip-verify"]);
            run_node_maintenance(&app_root, &node_args)
        }
        "cache-summary" => run_powershell(
            &app_root,
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                path_as_str(&maintenance_script)?,
                "-ShowCacheSummary",
                "-NoPrompt",
                "-NoOpenSummary",
            ],
        ),
        "edit-course-links" => {
            let mut args = vec![
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                path_as_str(&maintenance_script)?,
                "-EditCourseLinks",
            ];
            args.extend(back_args.iter().copied());
            run_powershell(&app_root, &args)
        }
        "laymans-diagnosis" => run_powershell(
            &app_root,
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                path_as_str(&maintenance_script)?,
                "-ShowLaymansDiagnosis",
                "-NoPrompt",
            ],
        ),
        "export-fact-check" => {
            ensure_node_toolchain()?;
            run_node_script(
                &app_root,
                &scripts.join("planner/export-transfer-planner-fact-check.cjs"),
                &[],
            )
        }
        "help" | "-h" | "--help" => {
            print_help(LauncherKind::CoursePlanner);
            Ok(0)
        }
        _ => {
            eprintln!("Unknown course planner mode \"{mode}\".\n");
            print_help(LauncherKind::CoursePlanner);
            Ok(0)
        }
    }
}

fn run_powershell(working_directory: &Path, args: &[&str]) -> LauncherResult<i32> {
    let executable = find_tool("powershell").unwrap_or_else(|| PathBuf::from("powershell.exe"));
    match Command::new(executable)
        .args(args)
        .current_dir(working_directory)
        .status()
    {
        Ok(status) => Ok(status_code(status)),
        Err(error) => {
            eprintln!("Could not start PowerShell: {error}");
            Ok(1)
        }
    }
}

fn run_node_maintenance(app_root: &Path, args: &[&str]) -> LauncherResult<i32> {
    ensure_node_toolchain()?;
    run_node_script(
        app_root,
        &app_root.join("scripts/run-transfer-planner-maintenance.cjs"),
        args,
    )
}

fn run_node_script(app_root: &Path, script: &Path, args: &[&str]) -> LauncherResult<i32> {
    let node = ensure_node_toolchain()?;
    let status = Command::new(node)
        .arg(script)
        .args(args)
        .current_dir(app_root)
        .status()?;
    Ok(status_code(status))
}

fn finish_planner_action(
    repo_root: &Path,
    label: &str,
    code: i32,
    interactive: bool,
) -> LauncherResult<()> {
    let app_root = repo_root.join("source");
    organize_tmp_artifacts(&app_root);
    println!();
    if code == 0 {
        println!("{label} finished successfully.");
    } else {
        println!("{label} failed with exit code {code}.");
    }
    println!();
    pause();
    if !interactive {
        return Ok(());
    }
    Ok(())
}

fn organize_tmp_artifacts(app_root: &Path) {
    let node = find_tool("node");
    let script = app_root.join("scripts/organize-tmp-artifacts.cjs");
    if let (Some(node), true) = (node, script.is_file()) {
        let _ = Command::new(node)
            .arg(script)
            .arg("--quiet")
            .current_dir(app_root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn pause() {
    print!("Press Enter to continue...");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

fn prompt(message: &str) -> LauncherResult<String> {
    print!("{message}");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(answer)
}

fn run_resource_launcher(repo_root: &Path, args: &[OsString]) -> LauncherResult<i32> {
    let app_root = repo_root.join("source");
    let script = app_root.join("scripts/assets/add-catalog-item.cjs");
    if !script.is_file() {
        return Err(format!("Could not find {}.", script.display()).into());
    }
    let node = find_tool("node").ok_or_else(|| {
        "Node.js is required for the resource editor. Install Node.js LTS and run this launcher again.".to_string()
    })?;
    let status = Command::new(node)
        .arg(&script)
        .args(args)
        .current_dir(&app_root)
        .status()?;
    organize_tmp_artifacts(&app_root);
    Ok(status_code(status))
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn path_as_str(path: &Path) -> LauncherResult<&str> {
    path.to_str()
        .ok_or_else(|| format!("Path is not valid Unicode: {}", path.display()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_executable_names_select_the_expected_mode() {
        assert_eq!(
            launcher_kind(OsStr::new("Start-to-run.exe")),
            LauncherKind::Startup
        );
        assert_eq!(
            launcher_kind(OsStr::new("add-or-remove-resources.exe")),
            LauncherKind::Resources
        );
        assert_eq!(
            launcher_kind(OsStr::new("Course-Planner-Updater.exe")),
            LauncherKind::CoursePlanner
        );
        assert_eq!(
            launcher_kind(OsStr::new("gatorguide-launcher.exe")),
            LauncherKind::CoursePlanner
        );
    }

    #[test]
    fn startup_mode_environment_aliases_match_the_legacy_launcher() {
        for normal in ["1", "run", "normal", "NORMAL"] {
            assert_eq!(normalize_startup_mode(normal), Some(StartupMode::Normal));
        }
        for demo in ["2", "demo", "DEMO"] {
            assert_eq!(normalize_startup_mode(demo), Some(StartupMode::Demo));
        }
        assert_eq!(normalize_startup_mode("invalid"), None);
    }

    #[test]
    fn demo_environment_enables_all_legacy_switches() {
        assert_eq!(
            startup_environment(StartupMode::Demo),
            vec![
                ("GATORGUIDE_STARTUP_MODE", "demo"),
                ("GATORGUIDE_DEMO_MODE", "1"),
                ("EXPO_PUBLIC_GATORGUIDE_DEMO_MODE", "1"),
            ]
        );
    }

    #[test]
    fn normal_environment_clears_inherited_demo_switches() {
        assert_eq!(
            startup_environment(StartupMode::Normal),
            vec![
                ("GATORGUIDE_STARTUP_MODE", "normal"),
                ("GATORGUIDE_DEMO_MODE", ""),
                ("EXPO_PUBLIC_GATORGUIDE_DEMO_MODE", ""),
            ]
        );
    }

    #[test]
    fn compatibility_executable_can_find_the_repository_root() {
        let temp_root = env::temp_dir().join(format!(
            "gatorguide-launcher-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after epoch")
                .as_nanos()
        ));
        let compatibility_dir = temp_root.join("source/scripts");
        fs::create_dir_all(&compatibility_dir).expect("create temporary repository layout");
        fs::create_dir_all(temp_root.join("source")).expect("create app directory");
        fs::write(temp_root.join("source/package.json"), "{}")
            .expect("write source package marker");

        assert_eq!(
            find_repository_root(&compatibility_dir),
            Some(temp_root.clone())
        );
        fs::remove_dir_all(temp_root).expect("remove temporary repository layout");
    }
}
