//! Languages the coding agent can build and run, and the sandbox every run
//! lives in. A project is one folder under `<data>/projects/<uuid>`; a run can
//! read the toolchain, the system and that folder, write only inside that
//! folder, and never use the network. Toolchain locations (Go, the rustup
//! toolchain, Anaconda's Python, Xcode's clang and SDK, Lua, Node) are resolved
//! once outside the sandbox and cached, so a run never waits on `xcrun`.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Fetching dependencies may take this long; a build (which may compile them)
/// this long; a run or a test run less.
pub const DEPS_TIMEOUT: Duration = Duration::from_secs(600);
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(300);
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);
pub const TEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Longest stdout or stderr kept from one process, in bytes.
pub const OUTPUT_CAP: usize = 64 * 1024;
/// Longest stdin handed to a program, in bytes.
pub const STDIN_CAP: usize = 256 * 1024;
/// Hidden folder inside a project for caches, temporary files and built binaries.
pub const CACHE_DIR: &str = ".cache";

pub const LANGUAGES: [&str; 6] = ["rust", "go", "python", "c", "lua", "typescript"];

pub fn language_name(id: &str) -> &'static str {
    match id {
        "rust" => "Rust",
        "go" => "Go",
        "python" => "Python",
        "c" => "C",
        "lua" => "Lua",
        "typescript" => "TypeScript",
        _ => "Unknown",
    }
}
pub fn is_language(id: &str) -> bool {
    LANGUAGES.contains(&id)
}
/// The file a project starts from, relative to its folder.
pub fn entry_file(language: &str) -> &'static str {
    match language {
        "rust" => "src/main.rs",
        "go" => "main.go",
        "python" => "main.py",
        "c" => "main.c",
        "lua" => "main.lua",
        "typescript" => "main.ts",
        _ => "main.txt",
    }
}
/// The files a new project starts with: a hello-world shaped like a real
/// project in that language (a Cargo crate, a Go module, …), each with a
/// README and a `.gitignore` for the cache folder.
pub fn scaffold(language: &str, name: &str) -> Vec<(&'static str, String)> {
    let readme = format!(
        "# {name}\n\nMade with Invisible AI's coding agent ({}).\n",
        language_name(language)
    );
    let ignore = format!("{CACHE_DIR}/\ntarget/\nnode_modules/\n");
    let mut files = vec![("README.md", readme), (".gitignore", ignore)];
    match language {
        "rust" => {
            files.push((
                "Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n".into(),
            ));
            files.push(("src/main.rs", "fn main() {\n    println!(\"Hello, world!\");\n}\n".into()));
        }
        "go" => {
            files.push(("go.mod", "module app\n\ngo 1.21\n".into()));
            files.push((
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"Hello, world!\")\n}\n".into(),
            ));
        }
        "python" => files.push(("main.py", "def main():\n    print(\"Hello, world!\")\n\n\nif __name__ == \"__main__\":\n    main()\n".into())),
        "c" => files.push((
            "main.c",
            "#include <stdio.h>\n\nint main(void) {\n    printf(\"Hello, world!\\n\");\n    return 0;\n}\n".into(),
        )),
        "lua" => files.push(("main.lua", "print(\"Hello, world!\")\n".into())),
        "typescript" => files.push((
            "main.ts",
            "function main(): void {\n  console.log(\"Hello, world!\");\n}\n\nmain();\n".into(),
        )),
        _ => {}
    }
    files
}

/// Where each tool was found. `None` means that language is unavailable and
/// the UI says so instead of failing a run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Detected {
    pub go: Option<PathBuf>,
    /// The rustup toolchain root (`…/toolchains/stable-aarch64-apple-darwin`).
    pub rust_toolchain: Option<PathBuf>,
    pub python: Option<PathBuf>,
    pub clang: Option<PathBuf>,
    pub sdk: Option<PathBuf>,
    pub lua: Option<PathBuf>,
    pub node: Option<PathBuf>,
    pub node_major: u32,
    /// npm beside node, for `package.json` dependencies.
    pub npm: Option<PathBuf>,
    /// Whether Anaconda's Python can `import pytest`; otherwise unittest runs tests.
    pub pytest: bool,
    /// macOS with `/usr/bin/sandbox-exec`: the only platform runs are allowed on.
    pub sandbox: bool,
}
impl Detected {
    pub fn available(&self, language: &str) -> bool {
        self.sandbox
            && match language {
                "rust" => self.rust_toolchain.is_some() && self.clang.is_some(),
                "go" => self.go.is_some(),
                "python" => self.python.is_some(),
                "c" => self.clang.is_some() && self.sdk.is_some(),
                "lua" => self.lua.is_some(),
                "typescript" => self.node.is_some() && self.node_major >= 22,
                _ => false,
            }
    }
    /// What the UI shows beside each language: the tool in use, or what is missing.
    pub fn detail(&self, language: &str) -> String {
        if !self.sandbox {
            return "Runs need the macOS sandbox on the backend".into();
        }
        let shown = |p: &Option<PathBuf>, missing: &str| {
            p.as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| missing.to_owned())
        };
        match language {
            "rust" => shown(&self.rust_toolchain, "Install Rust with rustup"),
            "go" => shown(&self.go, "Install Go (brew install go)"),
            "python" => shown(&self.python, "Install Anaconda"),
            "c" => shown(&self.clang, "Install Xcode or its command line tools"),
            "lua" => shown(&self.lua, "Install Lua (brew install luajit)"),
            "typescript" => {
                if self.node.is_some() && self.node_major < 22 {
                    "Node.js 22 or newer is needed to run TypeScript directly".into()
                } else {
                    shown(&self.node, "Install Node.js 22 or newer")
                }
            }
            _ => String::new(),
        }
    }
    pub fn languages(&self) -> Vec<Value> {
        LANGUAGES
            .iter()
            .map(|id| json!({"id":id,"name":language_name(id),"available":self.available(id),"detail":self.detail(id),"entry":entry_file(id)}))
            .collect()
    }
}

/// Resolved once per process, lazily, outside the sandbox.
#[derive(Default)]
pub struct Toolchains {
    detected: tokio::sync::OnceCell<Detected>,
}
impl Toolchains {
    pub async fn get(&self) -> &Detected {
        self.detected.get_or_init(detect).await
    }
}

fn which(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend_from_slice(extra);
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs.into_iter()
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}
async fn output_of(program: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(program)
            .args(args)
            .env("RUSTUP_AUTO_INSTALL", "0")
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
/// Anaconda first (`CONDA_EXE`, `CONDA_PREFIX`, the usual install folders), as
/// its Python carries the scientific libraries; `INVISIBLE_PYTHON` overrides.
fn find_python() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("INVISIBLE_PYTHON").filter(|v| !v.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut candidates = vec![];
    if let Some(exe) = std::env::var_os("CONDA_EXE") {
        // `…/anaconda3/bin/conda` lives beside the base interpreter.
        if let Some(bin) = Path::new(&exe).parent() {
            candidates.push(bin.join("python3"));
        }
    }
    if let Some(prefix) = std::env::var_os("CONDA_PREFIX") {
        candidates.push(Path::new(&prefix).join("bin/python3"));
    }
    for root in [
        "/opt/anaconda3",
        "/opt/homebrew/anaconda3",
        "/usr/local/anaconda3",
        "/opt/miniconda3",
    ] {
        candidates.push(Path::new(root).join("bin/python3"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        for folder in ["anaconda3", "miniconda3", "miniforge3"] {
            candidates.push(Path::new(&home).join(folder).join("bin/python3"));
        }
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}
async fn detect() -> Detected {
    let mut d = Detected {
        sandbox: cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").is_file(),
        ..Default::default()
    };
    d.go = which("go", &[]);
    d.python = find_python();
    d.lua = which("luajit", &[])
        .or_else(|| which("lua", &[]))
        .or_else(|| which("lua5.4", &[]));
    d.node = which("node", &[]);
    d.npm = d
        .node
        .as_ref()
        .and_then(|n| n.parent())
        .map(|dir| dir.join("npm"))
        .filter(|p| p.is_file())
        .or_else(|| which("npm", &[]));
    if let Some(python) = &d.python {
        d.pytest = output_of(python, &["-c", "import pytest"]).await.is_some();
    }
    if let Some(node) = &d.node
        && let Some(version) = output_of(node, &["--version"]).await
    {
        d.node_major = version
            .trim_start_matches('v')
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
    }
    if let Some(rustup) = which(
        "rustup",
        &[Path::new(&std::env::var_os("HOME").unwrap_or_default()).join(".cargo/bin")],
    ) && crate::agent::compiler_available()
        && let Some(rustc) = output_of(&rustup, &["which", "rustc"]).await
        && let Ok(rustc) = PathBuf::from(rustc).canonicalize()
    {
        d.rust_toolchain = rustc.parent().and_then(Path::parent).map(Path::to_path_buf);
    }
    let xcrun = Path::new("/usr/bin/xcrun");
    if xcrun.is_file() {
        d.clang = output_of(xcrun, &["-f", "clang"])
            .await
            .map(PathBuf::from)
            .and_then(|p| p.canonicalize().ok());
        d.sdk = std::env::var_os("SDKROOT")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or(output_of(xcrun, &["--show-sdk-path"])
                .await
                .map(PathBuf::from));
    }
    d
}

/// The top of the tree a tool lives in, so the whole toolchain is readable:
/// `/opt/homebrew`, `/opt/anaconda3`, `~/.rustup`, …
fn root_of(path: &Path) -> PathBuf {
    let parts: Vec<_> = path.components().collect();
    let depth = if path.starts_with("/Users") || path.starts_with("/home") {
        4
    } else {
        3
    };
    parts.iter().take(depth).collect()
}

/// What a run produced. `phase` says whether the build or the program itself
/// stopped it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub success: bool,
    pub phase: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}
impl RunResult {
    /// The text a model or a person reads: output, errors, and how it ended.
    pub fn summary(&self) -> String {
        let mut text = String::new();
        let outcome = if self.timed_out {
            format!("{} timed out", self.phase)
        } else if self.success {
            format!("{} succeeded (exit code 0)", self.phase)
        } else {
            format!(
                "{} failed (exit code {})",
                self.phase,
                self.exit_code
                    .map(|c| c.to_string())
                    .unwrap_or("signal".into())
            )
        };
        text.push_str(&outcome);
        if !self.stdout.trim().is_empty() {
            text.push_str("\n\nstdout:\n");
            text.push_str(self.stdout.trim_end());
        }
        if !self.stderr.trim().is_empty() {
            text.push_str("\n\nstderr:\n");
            text.push_str(self.stderr.trim_end());
        }
        text
    }
}

/// One command to run inside the sandbox.
struct Step {
    phase: &'static str,
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    timeout: Duration,
}

fn join_path(dirs: &[PathBuf]) -> String {
    std::env::join_paths(
        dirs.iter()
            .chain([&PathBuf::from("/usr/bin"), &PathBuf::from("/bin")]),
    )
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_else(|_| "/usr/bin:/bin".into())
}

/// The file that declares a project's dependencies, when the language has a
/// package manager the agent may use: Cargo, Go modules, pip, npm.
pub fn manifest(language: &str) -> Option<&'static str> {
    match language {
        "rust" => Some("Cargo.toml"),
        "go" => Some("go.mod"),
        "python" => Some("requirements.txt"),
        "typescript" => Some("package.json"),
        _ => None,
    }
}
/// What the agent is told about dependencies in this language.
pub fn dependency_notes(language: &str) -> &'static str {
    match language {
        "rust" => {
            "Dependencies: add crates under [dependencies] in Cargo.toml; the backend runs `cargo fetch` (network only for that step) and then builds offline."
        }
        "go" => {
            "Dependencies: add `require` lines to go.mod (or import the module and the backend runs `go mod tidy`); modules are downloaded in a separate step, then the build runs offline."
        }
        "python" => {
            "Dependencies: list pip packages in requirements.txt (one per line); the backend installs them into a virtualenv inside the project (network only for that step). Anaconda's own libraries (numpy, pandas, …) are importable without it."
        }
        "typescript" => {
            "Dependencies: declare them in package.json under dependencies; the backend runs `npm install` into the project's node_modules (network only for that step). Use ESM imports."
        }
        "lua" => "Dependencies: none; LuaJIT with its standard libraries only.",
        _ => "Dependencies: none; the C standard library and libm only.",
    }
}
/// Whether the manifest declares anything to fetch.
pub fn wants_dependencies(language: &str, root: &Path) -> bool {
    let Some(file) = manifest(language) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(root.join(file)) else {
        return false;
    };
    match language {
        "rust" => {
            // Any entry under a [dependencies]-like table.
            let mut in_deps = false;
            text.lines().any(|line| {
                let line = line.trim();
                if line.starts_with('[') {
                    in_deps = line.contains("dependencies");
                    false
                } else {
                    in_deps && !line.is_empty() && !line.starts_with('#')
                }
            })
        }
        "go" => text.lines().any(|l| l.trim_start().starts_with("require")),
        "python" => text
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')),
        "typescript" => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                ["dependencies", "devDependencies"]
                    .iter()
                    .find_map(|k| v[k].as_object().filter(|o| !o.is_empty()).map(|_| ()))
            })
            .is_some(),
        _ => false,
    }
}
/// A fingerprint of the manifest and its lock file, so dependencies are
/// fetched again only when they change.
fn dependency_stamp(language: &str, root: &Path) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let locks: &[&str] = match language {
        "rust" => &["Cargo.toml", "Cargo.lock"],
        "go" => &["go.mod", "go.sum"],
        "python" => &["requirements.txt"],
        "typescript" => &["package.json", "package-lock.json"],
        _ => &[],
    };
    for file in locks {
        hasher.update(file.as_bytes());
        hasher.update(std::fs::read(root.join(file)).unwrap_or_default());
    }
    format!("{:x}", hasher.finalize())
}
/// Where fetched dependencies land; missing means a fetch is due.
fn dependency_home(language: &str, root: &Path) -> PathBuf {
    let cache = root.join(CACHE_DIR);
    match language {
        "rust" => cache.join("cargo/registry"),
        "go" => cache.join("gopath/pkg/mod"),
        "python" => cache.join("venv"),
        "typescript" => root.join("node_modules"),
        _ => cache,
    }
}
/// The environment every tool shares inside a project.
fn common_env(root: &Path, mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    let cache = root.join(CACHE_DIR);
    env.push((
        "HOME".into(),
        cache.join("home").to_string_lossy().into_owned(),
    ));
    env.push((
        "TMPDIR".into(),
        cache.join("tmp").to_string_lossy().into_owned(),
    ));
    env.push(("LANG".into(), "C.UTF-8".into()));
    env.push(("TERM".into(), "dumb".into()));
    env.push(("NO_COLOR".into(), "1".into()));
    env
}
/// The language's base environment: tool on PATH, every cache inside the
/// project. `offline` is false only for the dependency step.
fn language_env(d: &Detected, language: &str, root: &Path, offline: bool) -> Vec<(String, String)> {
    let cache = root.join(CACHE_DIR);
    let s = |p: &Path| p.to_string_lossy().into_owned();
    match language {
        "rust" => {
            let toolchain = d.rust_toolchain.clone().unwrap();
            let mut env = vec![
                ("PATH".into(), join_path(&[toolchain.join("bin")])),
                ("CARGO_HOME".into(), s(&cache.join("cargo"))),
                ("CARGO_TARGET_DIR".into(), s(&root.join("target"))),
                (
                    "RUSTFLAGS".into(),
                    format!("-C linker={}", s(d.clang.as_ref().unwrap())),
                ),
                ("CARGO_TERM_COLOR".into(), "never".into()),
                ("CARGO_NET_GIT_FETCH_WITH_CLI".into(), "false".into()),
            ];
            if offline {
                env.push(("CARGO_NET_OFFLINE".into(), "true".into()));
            }
            if let Some(home) = std::env::var_os("RUSTUP_HOME") {
                env.push(("RUSTUP_HOME".into(), home.to_string_lossy().into_owned()));
            }
            if let Some(sdk) = &d.sdk {
                env.push(("SDKROOT".into(), s(sdk)));
            }
            env
        }
        "go" => {
            let go = d.go.clone().unwrap();
            let mut env = vec![
                (
                    "PATH".into(),
                    join_path(&[go.parent().unwrap().to_path_buf()]),
                ),
                ("GOCACHE".into(), s(&cache.join("go-build"))),
                ("GOPATH".into(), s(&cache.join("gopath"))),
                ("GOMODCACHE".into(), s(&cache.join("gopath/pkg/mod"))),
                ("GOFLAGS".into(), "-mod=mod".into()),
                ("GOTOOLCHAIN".into(), "local".into()),
                ("GOTELEMETRY".into(), "off".into()),
                ("CGO_ENABLED".into(), "0".into()),
            ];
            if offline {
                env.push(("GOPROXY".into(), "off".into()));
                env.push(("GOSUMDB".into(), "off".into()));
            }
            env
        }
        "python" => {
            let python = d.python.clone().unwrap();
            vec![
                (
                    "PATH".into(),
                    join_path(&[
                        cache.join("venv/bin"),
                        python.parent().unwrap().to_path_buf(),
                    ]),
                ),
                ("PYTHONDONTWRITEBYTECODE".into(), "1".into()),
                ("PYTHONNOUSERSITE".into(), "1".into()),
                ("PYTHONUNBUFFERED".into(), "1".into()),
                ("PYTHONIOENCODING".into(), "utf-8".into()),
                ("PIP_CACHE_DIR".into(), s(&cache.join("pip"))),
                ("PIP_DISABLE_PIP_VERSION_CHECK".into(), "1".into()),
                ("MPLCONFIGDIR".into(), s(&cache.join("matplotlib"))),
                ("MPLBACKEND".into(), "Agg".into()),
                ("XDG_CACHE_HOME".into(), s(&cache)),
            ]
        }
        "typescript" => {
            let node = d.node.clone().unwrap();
            vec![
                (
                    "PATH".into(),
                    join_path(&[
                        root.join("node_modules/.bin"),
                        node.parent().unwrap().to_path_buf(),
                    ]),
                ),
                ("NODE_OPTIONS".into(), String::new()),
                ("NODE_DISABLE_COLORS".into(), "1".into()),
                ("NODE_COMPILE_CACHE".into(), s(&cache.join("node"))),
                ("npm_config_cache".into(), s(&cache.join("npm"))),
                ("npm_config_update_notifier".into(), "false".into()),
                ("npm_config_fund".into(), "false".into()),
                ("npm_config_audit".into(), "false".into()),
                ("npm_config_loglevel".into(), "error".into()),
            ]
        }
        "c" => d
            .sdk
            .as_ref()
            .map(|sdk| vec![("SDKROOT".to_string(), s(sdk))])
            .unwrap_or_default(),
        _ => vec![],
    }
}
/// The language environment for a tool working in the folder (Codex, Claude
/// Code): the toolchain on PATH and every cache inside the project.
pub fn language_env_for(
    d: &Detected,
    language: &str,
    root: &Path,
    offline: bool,
) -> Vec<(String, String)> {
    common_env(root, language_env(d, language, root, offline))
}
/// The Python that runs a project: its virtualenv once dependencies were
/// installed, Anaconda's interpreter otherwise.
fn project_python(d: &Detected, root: &Path) -> PathBuf {
    let venv = root.join(CACHE_DIR).join("venv/bin/python");
    if venv.is_file() {
        venv
    } else {
        d.python.clone().unwrap()
    }
}
/// The package-manager steps for a project, run with the network on and
/// every write still inside the project folder.
fn dependency_steps(d: &Detected, language: &str, root: &Path) -> Result<Vec<Step>> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let env = common_env(root, language_env(d, language, root, false));
    let step = |program: PathBuf, args: Vec<&str>| Step {
        phase: "deps",
        program,
        args: args.into_iter().map(String::from).collect(),
        env: env.clone(),
        timeout: DEPS_TIMEOUT,
    };
    Ok(match language {
        "rust" => vec![step(
            d.rust_toolchain.clone().unwrap().join("bin/cargo"),
            vec!["fetch"],
        )],
        "go" => {
            let go = d.go.clone().unwrap();
            vec![
                step(go.clone(), vec!["mod", "tidy"]),
                step(go, vec!["mod", "download"]),
            ]
        }
        "python" => {
            let venv = root.join(CACHE_DIR).join("venv");
            let mut steps = vec![];
            if !venv.join("bin/python").is_file() {
                steps.push(step(
                    d.python.clone().unwrap(),
                    vec!["-m", "venv", "--system-site-packages", &s(&venv)],
                ));
            }
            steps.push(Step {
                phase: "deps",
                program: venv.join("bin/python"),
                args: ["-m", "pip", "install", "-r", "requirements.txt"]
                    .iter()
                    .map(|a| a.to_string())
                    .collect(),
                env: env.clone(),
                timeout: DEPS_TIMEOUT,
            });
            steps
        }
        "typescript" => {
            let npm = d
                .npm
                .clone()
                .context("npm is not installed beside node on the backend")?;
            vec![step(npm, vec!["install", "--no-audit", "--no-fund"])]
        }
        other => bail!("{} has no package manager here", language_name(other)),
    })
}
/// The test step for a project: cargo test, go test, pytest or unittest, node --test.
fn test_step(d: &Detected, language: &str, root: &Path) -> Result<Step> {
    let env = common_env(root, language_env(d, language, root, true));
    let (program, args): (PathBuf, Vec<&str>) = match language {
        "rust" => (
            d.rust_toolchain.clone().unwrap().join("bin/cargo"),
            vec!["test", "--offline", "-q"],
        ),
        "go" => (d.go.clone().unwrap(), vec!["test", "./..."]),
        "python" => (
            project_python(d, root),
            if d.pytest {
                vec!["-m", "pytest", "-q", "-p", "no:cacheprovider"]
            } else {
                vec!["-m", "unittest", "discover", "-v"]
            },
        ),
        "typescript" => (
            d.node.clone().unwrap(),
            if d.node_major == 22 {
                vec!["--experimental-strip-types", "--test"]
            } else {
                vec!["--test"]
            },
        ),
        other => bail!(
            "{} has no test runner here; run the program instead",
            language_name(other)
        ),
    };
    Ok(Step {
        phase: "test",
        program,
        args: args.into_iter().map(String::from).collect(),
        env,
        timeout: TEST_TIMEOUT,
    })
}

/// The build step (if the language has one) and the run step for a project.
fn steps(d: &Detected, language: &str, root: &Path, args: &[String]) -> Result<Vec<Step>> {
    ensure!(
        d.available(language),
        "{} is not available on this backend: {}",
        language_name(language),
        d.detail(language)
    );
    let cache = root.join(CACHE_DIR);
    let bin = cache.join("bin");
    for dir in [cache.join("tmp"), bin.clone(), cache.join("home")] {
        std::fs::create_dir_all(dir)?;
    }
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let env = common_env(root, language_env(d, language, root, true));
    let run = |program: PathBuf, mut run_args: Vec<String>| {
        run_args.extend(args.iter().cloned());
        Step {
            phase: "run",
            program,
            args: run_args,
            env: env.clone(),
            timeout: RUN_TIMEOUT,
        }
    };
    let build = |program: PathBuf, build_args: Vec<String>| Step {
        phase: "build",
        program,
        args: build_args,
        env: env.clone(),
        timeout: BUILD_TIMEOUT,
    };
    Ok(match language {
        "rust" => {
            let toolchain = d.rust_toolchain.clone().unwrap();
            vec![
                build(
                    toolchain.join("bin/cargo"),
                    vec!["build".into(), "--offline".into(), "-q".into()],
                ),
                run(root.join("target/debug/app"), vec![]),
            ]
        }
        "go" => {
            let go = d.go.clone().unwrap();
            vec![
                // Xcode's git shim cannot write its cache in the sandbox, so
                // the module's repository is left out of the build stamp.
                build(
                    go,
                    vec![
                        "build".into(),
                        "-buildvcs=false".into(),
                        "-o".into(),
                        s(&bin.join("app")),
                        ".".into(),
                    ],
                ),
                run(bin.join("app"), vec![]),
            ]
        }
        "python" => vec![run(project_python(d, root), vec!["main.py".into()])],
        "c" => {
            let clang = d.clang.clone().unwrap();
            let sdk = d.sdk.clone().unwrap();
            let mut sources: Vec<String> = std::fs::read_dir(root)?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".c"))
                .collect();
            sources.sort();
            ensure!(!sources.is_empty(), "No .c source files in the project");
            let mut build_args = vec![
                "-isysroot".to_string(),
                s(&sdk),
                "-O1".into(),
                "-Wall".into(),
                "-fdiagnostics-color=never".into(),
                "-o".into(),
                s(&bin.join("app")),
            ];
            build_args.extend(sources);
            build_args.push("-lm".into());
            vec![build(clang, build_args), run(bin.join("app"), vec![])]
        }
        "lua" => vec![run(d.lua.clone().unwrap(), vec!["main.lua".into()])],
        "typescript" => {
            let mut node_args = vec!["--no-warnings".to_string()];
            if d.node_major == 22 {
                node_args.push("--experimental-strip-types".into());
            }
            node_args.push("main.ts".into());
            vec![run(d.node.clone().unwrap(), node_args)]
        }
        _ => bail!("Unknown language"),
    })
}

pub(crate) fn quote(p: &Path) -> String {
    serde_json::to_string(&p.to_string_lossy()).unwrap()
}
/// Roots every run may read: the project, the toolchains and the system.
fn readable_roots(d: &Detected, project: &Path) -> Vec<String> {
    let mut roots = vec![
        quote(project),
        "\"/usr\"".into(),
        "\"/bin\"".into(),
        "\"/sbin\"".into(),
        "\"/System\"".into(),
        "\"/Library/Apple\"".into(),
        "\"/Library/Developer\"".into(),
        "\"/Library/Preferences/Logging\"".into(),
        "\"/private/var/db\"".into(),
        "\"/private/var/select\"".into(),
        "\"/private/etc\"".into(),
        "\"/dev\"".into(),
    ];
    for tool in [
        &d.go,
        &d.rust_toolchain,
        &d.python,
        &d.clang,
        &d.sdk,
        &d.lua,
        &d.node,
    ]
    .into_iter()
    .flatten()
    {
        roots.push(quote(&root_of(tool)));
    }
    if let Some(home) = std::env::var_os("RUSTUP_HOME") {
        roots.push(quote(Path::new(&home)));
    }
    for key in ["SDKROOT", "DEVELOPER_DIR"] {
        if let Some(value) = std::env::var_os(key) {
            roots.push(quote(Path::new(&value)));
        }
    }
    roots.sort();
    roots.dedup();
    roots
}
/// Seatbelt profile for building and running a project: no network, writes
/// only inside the project folder, reads only from the allow-list.
pub fn run_profile(d: &Detected, project: &Path) -> String {
    project_profile(d, project, false)
}
/// The same confinement with the network on: only the package-manager step
/// (`cargo fetch`, `go mod download`, `pip install`, `npm install`) gets it.
pub fn deps_profile(d: &Detected, project: &Path) -> String {
    project_profile(d, project, true)
}
fn project_profile(d: &Detected, project: &Path, network: bool) -> String {
    let readable = readable_roots(d, project)
        .iter()
        .map(|p| format!("(subpath {p})"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "(version 1)(allow default){}(deny file-read-data (require-not (require-any (literal \"/\") {readable})))(deny file-write* (require-not (require-any (subpath {}) (literal \"/dev/null\") (regex #\"^/dev/tty\"))))",
        if network { "" } else { "(deny network*)" },
        quote(project)
    ) + &format!(
        "(deny file-write* (subpath {}))",
        quote(&project.join(".git"))
    )
}
/// Seatbelt profile for Codex and Claude Code working in a project: network on
/// (they talk to their services), reads limited to the project, the toolchains,
/// the system and the tools' own state, writes limited to the project and that
/// state. The user's other files stay out of reach.
pub fn cli_profile(d: &Detected, project: &Path) -> String {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let uid = user_id();
    let mut readable = readable_roots(d, project);
    let mut writable = vec![
        quote(project),
        quote(&PathBuf::from(format!("/private/tmp/claude-{uid}"))),
    ];
    // The user's own temp folder (`/private/var/folders/…/T`): the tools'
    // shells put heredocs and scratch files there. Private to this user.
    if let Some(tmp) = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.starts_with("/private/var/folders"))
    {
        readable.push(quote(&tmp));
        writable.push(quote(&tmp));
    }
    for (p, rw) in [
        (home.join(".claude"), true),
        (home.join(".codex"), true),
        (home.join(".local"), false),
        (home.join(".cargo"), false),
        (home.join(".rustup"), false),
        (home.join("Library/Keychains"), false),
        (PathBuf::from("/opt"), false),
        (PathBuf::from("/Library"), false),
        (PathBuf::from("/Applications/Xcode.app"), false),
        (PathBuf::from(format!("/private/tmp/claude-{uid}")), false),
    ] {
        readable.push(quote(&p));
        if rw {
            writable.push(quote(&p));
        }
    }
    let readable_set = readable
        .iter()
        .map(|p| format!("(subpath {p})"))
        .chain([format!("(literal {})", quote(&home.join(".claude.json")))])
        .collect::<Vec<_>>()
        .join(" ");
    let writable_set = writable
        .iter()
        .map(|p| format!("(subpath {p})"))
        .chain([
            format!("(literal {})", quote(&home.join(".claude.json"))),
            "(subpath \"/dev\")".to_string(),
        ])
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "(version 1)(allow default)(deny file-read-data (require-not (require-any (literal \"/\") {readable_set})))(deny file-write* (require-not (require-any {writable_set})))"
    ) + &format!(
        "(deny file-write* (subpath {}))",
        quote(&project.join(".git"))
    )
}
/// The numeric user id, read from the environment of the running backend.
fn user_id() -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Some(home) = std::env::var_os("HOME")
            && let Ok(meta) = std::fs::metadata(home)
        {
            return meta.uid();
        }
    }
    501
}

/// Run one sandboxed process in `cwd`, feeding `stdin`, with bounded output
/// and a timeout that kills the whole process group (so `cargo run` cannot
/// leave the real program behind).
#[allow(clippy::too_many_arguments)] // Explicit process boundary: profile, program, IO and budget.
pub async fn run_sandboxed(
    profile: &str,
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    cwd: &Path,
    stdin: &str,
    timeout: Duration,
    phase: &str,
) -> Result<RunResult> {
    ensure!(
        cfg!(target_os = "macos"),
        "Programs run only on a macOS backend"
    );
    ensure!(stdin.len() <= STDIN_CAP, "Program input exceeds 256 KB");
    let start = Instant::now();
    let mut cmd = tokio::process::Command::new("/usr/bin/sandbox-exec");
    cmd.arg("-p")
        .arg(profile)
        .arg(program)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("Could not start {}", program.display()))?;
    let pid = child.id();
    let mut guard = GroupGuard::new(pid);
    let mut input = child.stdin.take().context("No stdin")?;
    let stdin = stdin.to_owned();
    tokio::spawn(async move {
        let _ = input.write_all(stdin.as_bytes()).await;
        let _ = input.shutdown().await;
    });
    let out = child.stdout.take().context("No stdout")?;
    let err = child.stderr.take().context("No stderr")?;
    async fn drain<R: tokio::io::AsyncRead + Unpin>(mut pipe: R) -> Result<(Vec<u8>, usize)> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut total = 0usize;
        loop {
            let n = pipe.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            total += n;
            if buf.len() < OUTPUT_CAP {
                buf.extend_from_slice(&chunk[..n.min(OUTPUT_CAP - buf.len())]);
            }
        }
        Ok((buf, total))
    }
    let waited = tokio::time::timeout(timeout, async {
        let (o, e, status) = tokio::join!(drain(out), drain(err), child.wait());
        Ok::<_, anyhow::Error>((o?, e?, status?))
    })
    .await;
    let (stdout, stderr, status, timed_out) = match waited {
        Ok(Ok((o, e, status))) => {
            guard.disarm();
            (o, e, Some(status), false)
        }
        Ok(Err(e)) => {
            kill_group(pid);
            let _ = child.kill().await;
            return Err(e);
        }
        Err(_) => {
            kill_group(pid);
            let _ = child.kill().await;
            let _ = child.wait().await;
            ((Vec::new(), 0), (Vec::new(), 0), None, true)
        }
    };
    let text = |(bytes, total): (Vec<u8>, usize)| {
        let mut s = String::from_utf8_lossy(&bytes).into_owned();
        if total > bytes.len() {
            s.push_str(&format!(
                "\n…[output truncated: {} of {total} bytes shown]",
                bytes.len()
            ));
        }
        s
    };
    Ok(RunResult {
        success: status.is_some_and(|s| s.success()),
        phase: phase.into(),
        exit_code: status.and_then(|s| s.code()),
        stdout: text(stdout),
        stderr: text(stderr),
        duration_ms: start.elapsed().as_millis() as u64,
        timed_out,
    })
}
/// Kills the whole process group when dropped, so an aborted job (Stop,
/// delete, a panic) cannot leave `cargo`, the program or a coding tool
/// running behind `sandbox-exec`. Disarmed once the process has exited.
pub struct GroupGuard(Option<u32>);
impl GroupGuard {
    pub fn new(pid: Option<u32>) -> Self {
        Self(pid)
    }
    pub fn disarm(&mut self) {
        self.0 = None;
    }
}
impl Drop for GroupGuard {
    fn drop(&mut self) {
        kill_group(self.0.take());
    }
}
/// Kill everything in the process group `sandbox-exec` leads.
pub fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-9", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Build (when the language needs it) and run a project with `stdin` and
/// `args`. A failed build is reported as the result; the program is not run.
pub async fn run_project(
    d: &Detected,
    language: &str,
    root: &Path,
    stdin: &str,
    args: &[String],
) -> Result<RunResult> {
    ensure!(
        args.len() <= 32 && args.iter().all(|a| a.len() <= 4096),
        "Too many or too long arguments"
    );
    let root = root.canonicalize().context("Project folder is missing")?;
    if let Some(failed) = fetch_dependencies(d, language, &root).await? {
        return Ok(failed);
    }
    let profile = run_profile(d, &root);
    let mut last = None;
    for step in steps(d, language, &root, args)? {
        let input = if step.phase == "run" { stdin } else { "" };
        let result = run_sandboxed(
            &profile,
            &step.program,
            &step.args,
            &step.env,
            &root,
            input,
            step.timeout,
            step.phase,
        )
        .await?;
        let stop = !result.success;
        last = Some(result);
        if stop {
            break;
        }
    }
    last.context("Nothing to run")
}
/// Fetch the manifest's dependencies when it changed since the last fetch (or
/// nothing was fetched yet). Returns the failed step, if any; `None` means the
/// project is ready to build.
pub async fn fetch_dependencies(
    d: &Detected,
    language: &str,
    root: &Path,
) -> Result<Option<RunResult>> {
    if !wants_dependencies(language, root) {
        return Ok(None);
    }
    let stamp_file = root.join(CACHE_DIR).join(format!("deps-{language}.sha256"));
    let stamp = dependency_stamp(language, root);
    if std::fs::read_to_string(&stamp_file).is_ok_and(|s| s.trim() == stamp)
        && dependency_home(language, root).exists()
    {
        return Ok(None);
    }
    for dir in [
        root.join(CACHE_DIR).join("tmp"),
        root.join(CACHE_DIR).join("home"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    let profile = deps_profile(d, root);
    for step in dependency_steps(d, language, root)? {
        let result = run_sandboxed(
            &profile,
            &step.program,
            &step.args,
            &step.env,
            root,
            "",
            step.timeout,
            step.phase,
        )
        .await?;
        if !result.success {
            let _ = std::fs::remove_file(&stamp_file);
            return Ok(Some(result));
        }
    }
    // The lock file may have changed during the fetch (Cargo.lock, go.sum, package-lock.json).
    std::fs::write(&stamp_file, dependency_stamp(language, root))?;
    Ok(None)
}
/// Run the project's tests in the sandbox (dependencies fetched first).
pub async fn run_tests(d: &Detected, language: &str, root: &Path) -> Result<RunResult> {
    let root = root.canonicalize().context("Project folder is missing")?;
    ensure!(
        d.available(language),
        "{} is not available on this backend",
        language_name(language)
    );
    if let Some(failed) = fetch_dependencies(d, language, &root).await? {
        return Ok(failed);
    }
    for dir in [
        root.join(CACHE_DIR).join("tmp"),
        root.join(CACHE_DIR).join("home"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    let step = test_step(d, language, &root)?;
    let profile = run_profile(d, &root);
    run_sandboxed(
        &profile,
        &step.program,
        &step.args,
        &step.env,
        &root,
        "",
        step.timeout,
        step.phase,
    )
    .await
}
/// The programs the agent may run by name inside a project (Claude Code has
/// Bash; here only the toolchain itself, with no shell): the resolved binary,
/// or an error naming what is allowed.
pub fn command_program(
    d: &Detected,
    language: &str,
    root: &Path,
    program: &str,
) -> Result<(PathBuf, Vec<String>)> {
    let venv = root.join(CACHE_DIR).join("venv/bin");
    let python = || -> Result<PathBuf> {
        let v = venv.join("python");
        if v.is_file() {
            Ok(v)
        } else {
            d.python
                .clone()
                .context("Python is not installed on the backend")
        }
    };
    let toolchain_bin = |name: &str| -> Result<PathBuf> {
        d.rust_toolchain
            .as_ref()
            .map(|t| t.join("bin").join(name))
            .filter(|p| p.is_file())
            .context("The Rust toolchain is not installed on the backend")
    };
    let beside = |p: &Option<PathBuf>, name: &str, what: &str| -> Result<PathBuf> {
        p.as_ref()
            .and_then(|p| p.parent())
            .map(|dir| dir.join(name))
            .filter(|p| p.is_file())
            .with_context(|| format!("{what} is not installed on the backend"))
    };
    let (path, prefix): (PathBuf, Vec<String>) = match program.trim() {
        "cargo" => (toolchain_bin("cargo")?, vec![]),
        "rustc" => (toolchain_bin("rustc")?, vec![]),
        "rustfmt" => (toolchain_bin("rustfmt")?, vec![]),
        "go" => (
            d.go.clone().context("Go is not installed on the backend")?,
            vec![],
        ),
        "gofmt" => (beside(&d.go, "gofmt", "gofmt")?, vec![]),
        "python" | "python3" => (python()?, vec![]),
        "pip" | "pip3" => (python()?, vec!["-m".into(), "pip".into()]),
        "pytest" => (python()?, vec!["-m".into(), "pytest".into()]),
        "node" => (
            d.node
                .clone()
                .context("Node.js is not installed on the backend")?,
            vec![],
        ),
        "npm" => (
            d.npm
                .clone()
                .context("npm is not installed on the backend")?,
            vec![],
        ),
        "npx" => (beside(&d.npm, "npx", "npx")?, vec![]),
        "luajit" | "lua" => (
            d.lua
                .clone()
                .context("Lua is not installed on the backend")?,
            vec![],
        ),
        "clang" | "cc" | "gcc" => (
            d.clang
                .clone()
                .context("clang is not installed on the backend")?,
            vec![],
        ),
        other => bail!(
            "`{other}` cannot be run here. Allowed programs: cargo, rustc, rustfmt, go, gofmt, python, pip, pytest, node, npm, npx, luajit, clang. There is no shell; use the file tools for files."
        ),
    };
    let _ = language;
    Ok((path, prefix))
}
/// Run one allow-listed toolchain program in the project sandbox (no
/// network, writes inside the folder), with the language's environment.
pub async fn run_command(
    d: &Detected,
    language: &str,
    root: &Path,
    program: &str,
    args: &[String],
    stdin: &str,
) -> Result<RunResult> {
    ensure!(d.sandbox, "Commands run only on a macOS backend");
    ensure!(
        args.len() <= 64 && args.iter().all(|a| a.len() <= 4096 && !a.contains('\0')),
        "Too many or too long arguments"
    );
    let root = root.canonicalize().context("Project folder is missing")?;
    for dir in [
        root.join(CACHE_DIR).join("tmp"),
        root.join(CACHE_DIR).join("home"),
        root.join(CACHE_DIR).join("bin"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    let (path, mut all_args) = command_program(d, language, &root, program)?;
    all_args.extend(args.iter().cloned());
    let env = common_env(&root, language_env(d, language, &root, true));
    let profile = run_profile(d, &root);
    run_sandboxed(
        &profile,
        &path,
        &all_args,
        &env,
        &root,
        stdin,
        Duration::from_secs(120),
        "command",
    )
    .await
}
/// Whether this language has a test runner the agent can call.
pub fn has_tests(language: &str) -> bool {
    matches!(language, "rust" | "go" | "python" | "typescript")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_an_entry_file_in_its_scaffold() {
        for language in LANGUAGES {
            let files = scaffold(language, "demo");
            assert!(
                files.iter().any(|(path, _)| *path == entry_file(language)),
                "{language}"
            );
            assert!(files.iter().any(|(path, _)| *path == ".gitignore"));
            assert!(is_language(language));
        }
        assert!(!is_language("brainfuck"));
    }

    #[test]
    fn manifests_say_whether_there_is_anything_to_fetch() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for language in LANGUAGES {
            for (path, content) in scaffold(language, "t") {
                let target = dir.path().join(path);
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::write(target, content)?;
            }
            assert!(
                !wants_dependencies(language, dir.path()),
                "{language}: a fresh scaffold needs nothing"
            );
        }
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"app\"\n\n[dependencies]\nitoa = \"1\"\n",
        )?;
        assert!(wants_dependencies("rust", dir.path()));
        std::fs::write(dir.path().join("requirements.txt"), "# nothing\n\n")?;
        assert!(!wants_dependencies("python", dir.path()));
        std::fs::write(dir.path().join("requirements.txt"), "six\n")?;
        assert!(wants_dependencies("python", dir.path()));
        std::fs::write(
            dir.path().join("package.json"),
            "{\"name\":\"app\",\"dependencies\":{\"nanoid\":\"5\"}}",
        )?;
        assert!(wants_dependencies("typescript", dir.path()));
        std::fs::write(
            dir.path().join("go.mod"),
            "module app\n\ngo 1.21\n\nrequire github.com/google/uuid v1.6.0\n",
        )?;
        assert!(wants_dependencies("go", dir.path()));
        assert!(!wants_dependencies("c", dir.path()) && !wants_dependencies("lua", dir.path()));
        let a = dependency_stamp("python", dir.path());
        std::fs::write(dir.path().join("requirements.txt"), "six\nattrs\n")?;
        assert_ne!(a, dependency_stamp("python", dir.path()));
        Ok(())
    }

    #[test]
    fn deps_profile_opens_the_network_and_nothing_else() {
        let d = Detected {
            sandbox: true,
            go: Some("/opt/homebrew/bin/go".into()),
            ..Default::default()
        };
        let project = Path::new("/tmp/projects/abc");
        let run = run_profile(&d, project);
        let deps = deps_profile(&d, project);
        assert!(run.contains("(deny network*)") && !deps.contains("(deny network*)"));
        assert_eq!(run.replace("(deny network*)", ""), deps);
    }

    /// Real package managers, real network: run with `cargo test -- --ignored deps`.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "downloads packages from the internet"]
    async fn dependencies_are_fetched_inside_the_project_and_programs_use_them() -> Result<()> {
        let d = detect().await;
        let cases: [(&str, &str, &str, &str); 4] = [
            (
                "rust",
                "Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nitoa = \"1\"\n",
                "fn main() { let mut b = itoa::Buffer::new(); println!(\"DEP {}\", b.format(42)); }\n",
            ),
            (
                "go",
                "go.mod",
                "module app\n\ngo 1.21\n\nrequire github.com/google/uuid v1.6.0\n",
                "package main\nimport (\"fmt\"; \"github.com/google/uuid\")\nfunc main() { fmt.Println(\"DEP\", len(uuid.NewString())) }\n",
            ),
            (
                "python",
                "requirements.txt",
                "six==1.17.0\n",
                "import six\nprint('DEP', six.__version__)\n",
            ),
            (
                "typescript",
                "package.json",
                "{\"name\":\"app\",\"private\":true,\"type\":\"module\",\"dependencies\":{\"nanoid\":\"5.1.5\"}}",
                "import { nanoid } from 'nanoid';\nconsole.log('DEP', nanoid().length);\n",
            ),
        ];
        for (language, manifest_name, manifest, program) in cases {
            if !d.available(language) {
                continue;
            }
            let dir = tempfile::tempdir()?;
            for (path, content) in scaffold(language, "t") {
                let target = dir.path().join(path);
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::write(target, content)?;
            }
            std::fs::write(dir.path().join(manifest_name), manifest)?;
            std::fs::write(dir.path().join(entry_file(language)), program)?;
            let run = run_project(&d, language, dir.path(), "", &[]).await?;
            assert!(run.success, "{language}: {}", run.summary());
            assert!(run.stdout.starts_with("DEP "), "{language}: {}", run.stdout);
            assert!(
                dependency_home(language, dir.path()).exists(),
                "{language}: dependencies live in the project"
            );
            let again = run_project(&d, language, dir.path(), "", &[]).await?;
            assert!(
                again.success && again.phase == "run",
                "{language}: {}",
                again.summary()
            );
        }
        Ok(())
    }

    #[test]
    fn roots_cover_the_toolchain_trees() {
        assert_eq!(
            root_of(Path::new("/opt/homebrew/Cellar/go/1.27/bin/go")),
            PathBuf::from("/opt/homebrew")
        );
        assert_eq!(
            root_of(Path::new("/opt/anaconda3/bin/python3.14")),
            PathBuf::from("/opt/anaconda3")
        );
        assert_eq!(
            root_of(Path::new("/Users/me/.rustup/toolchains/stable/bin/rustc")),
            PathBuf::from("/Users/me/.rustup")
        );
    }

    #[test]
    fn profiles_confine_writes_to_the_project_and_keep_the_home_out() {
        let d = Detected {
            sandbox: true,
            go: Some("/opt/homebrew/bin/go".into()),
            ..Default::default()
        };
        let project = Path::new("/tmp/projects/abc");
        let run = run_profile(&d, project);
        assert!(run.contains("(deny network*)"));
        assert!(run.contains(
            "(deny file-write* (require-not (require-any (subpath \"/tmp/projects/abc\")"
        ));
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(
            !run.contains(&format!("(subpath {})", quote(&home))),
            "the home folder itself is never readable"
        );
        assert!(!run.contains(".claude"));
        let cli = cli_profile(&d, project);
        assert!(
            !cli.contains("(deny network*)"),
            "the CLIs need their services"
        );
        assert!(cli.contains(".claude"));
        assert!(cli.contains("(deny file-read-data"));
    }

    #[test]
    fn run_results_summarize_both_streams() {
        let r = RunResult {
            success: false,
            phase: "build".into(),
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "error: x".into(),
            duration_ms: 3,
            timed_out: false,
        };
        let s = r.summary();
        assert!(s.starts_with("build failed (exit code 1)"));
        assert!(s.contains("stderr:\nerror: x"));
        assert!(!s.contains("stdout:"));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn every_available_language_runs_hello_world_and_cannot_read_outside() -> Result<()> {
        let d = detect().await;
        let home = tempfile::tempdir()?;
        let secret = home.path().join("private.txt");
        std::fs::write(&secret, "WALLET_SECRET_MUST_NOT_LEAK")?;
        // A program that prints the secret file if it can, and the network probe.
        let leak = |language: &str, path: &str| -> String {
            match language {
                "rust" => format!(
                    "fn main(){{ match std::fs::read_to_string({path:?}) {{ Ok(t)=>println!(\"LEAK {{t}}\"), Err(e)=>println!(\"denied {{e}}\") }} }}"
                ),
                "go" => format!(
                    "package main\nimport (\"fmt\";\"os\")\nfunc main(){{ b,err:=os.ReadFile({path:?}); if err!=nil {{ fmt.Println(\"denied\",err) }} else {{ fmt.Println(\"LEAK\",string(b)) }} }}"
                ),
                "python" => format!(
                    "try:\n    print('LEAK', open({path:?}).read())\nexcept Exception as e:\n    print('denied', e)\n"
                ),
                "c" => format!(
                    "#include <stdio.h>\nint main(void){{ FILE*f=fopen({path:?},\"r\"); if(!f){{ printf(\"denied\\n\"); return 0; }} char b[64]={{0}}; fread(b,1,63,f); printf(\"LEAK %s\\n\",b); return 0; }}"
                ),
                "lua" => format!(
                    "local f=io.open({path:?}) if f then print('LEAK '..f:read('*a')) else print('denied') end"
                ),
                _ => format!(
                    "import fs from 'node:fs';\ntry {{ console.log('LEAK', fs.readFileSync({path:?}, 'utf8')); }} catch (e) {{ console.log('denied', (e as Error).message); }}"
                ),
            }
        };
        let mut checked = 0;
        for language in LANGUAGES {
            if !d.available(language) {
                eprintln!("skipping {language}: {}", d.detail(language));
                continue;
            }
            let dir = tempfile::tempdir()?;
            for (path, content) in scaffold(language, "t") {
                let target = dir.path().join(path);
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::write(target, content)?;
            }
            // Real projects are git repositories; a build must not stumble on that.
            let _ = std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let hello = run_project(&d, language, dir.path(), "", &[]).await?;
            assert!(
                !dir.path().join("Library").exists(),
                "{language}: tool state must stay in the cache folder"
            );
            assert!(hello.success, "{language}: {}", hello.summary());
            assert_eq!(hello.stdout.trim(), "Hello, world!", "{language}");
            std::fs::write(
                dir.path().join(entry_file(language)),
                leak(language, &secret.to_string_lossy()),
            )?;
            let probe = run_project(&d, language, dir.path(), "", &[]).await?;
            assert!(
                !probe.summary().contains("WALLET_SECRET_MUST_NOT_LEAK"),
                "{language}: {}",
                probe.summary()
            );
            assert!(
                probe.stdout.contains("denied") || !probe.success,
                "{language}: {}",
                probe.summary()
            );
            checked += 1;
        }
        eprintln!("checked {checked} languages");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn stdin_reaches_the_program_and_timeouts_kill_it() -> Result<()> {
        let d = detect().await;
        if !d.available("python") {
            return Ok(());
        }
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("main.py"),
            "import sys\nprint(sys.stdin.read().upper(), sys.argv[1:])\n",
        )?;
        let r = run_project(&d, "python", dir.path(), "abc", &["x".into()]).await?;
        assert!(r.success, "{}", r.summary());
        assert_eq!(r.stdout.trim(), "ABC ['x']");
        let root = dir.path().canonicalize()?;
        let profile = run_profile(&d, &root);
        let slow = run_sandboxed(
            &profile,
            d.python.as_ref().unwrap(),
            &[
                "-c".into(),
                "import time\nprint('start', flush=True)\ntime.sleep(30)".into(),
            ],
            &[("HOME".into(), root.to_string_lossy().into_owned())],
            &root,
            "",
            Duration::from_millis(1500),
            "run",
        )
        .await?;
        assert!(slow.timed_out);
        assert!(!slow.success);
        // Aborting the task (Stop, delete) takes the process group down too.
        let marker = format!("guard-{}", std::process::id());
        let script = format!("import time\nprint('{marker}', flush=True)\ntime.sleep(60)");
        let profile2 = profile.clone();
        let python = d.python.clone().unwrap();
        let home = root.to_string_lossy().into_owned();
        let task = tokio::spawn(async move {
            run_sandboxed(
                &profile2,
                &python,
                &["-c".into(), script],
                &[("HOME".into(), home)],
                &root,
                "",
                Duration::from_secs(60),
                "run",
            )
            .await
        });
        let running = |marker: &str| {
            std::process::Command::new("/usr/bin/pgrep")
                .args(["-f", marker])
                .output()
                .map(|o| !o.stdout.trim_ascii().is_empty())
                .unwrap_or(false)
        };
        let mut seen = false;
        for _ in 0..50 {
            if running(&marker) {
                seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(seen, "the program should have started");
        task.abort();
        let _ = task.await;
        let mut gone = false;
        for _ in 0..50 {
            if !running(&marker) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "aborting the job must kill the sandboxed program");
        Ok(())
    }
}
