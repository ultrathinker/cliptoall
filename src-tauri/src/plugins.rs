use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use parking_lot::Mutex; // non-poisoning; used for PluginManagerState

#[cfg(windows)]
use windows::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows::Win32::System::JobObjects::*;

/// Wrapper around HANDLE to make it Send + Sync.
/// Job Object handles are safe to share across threads — they are kernel objects.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct SendHandle(HANDLE);
#[cfg(windows)]
unsafe impl Send for SendHandle {}
#[cfg(windows)]
unsafe impl Sync for SendHandle {}

// ── Protocol types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginFunction {
    pub id: String,
    pub label: String,
    pub default_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginHello {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub settings_description: String,
    #[serde(default)]
    pub settings_format: String,
    pub functions: Vec<PluginFunction>,
}

#[derive(Debug, Deserialize)]
pub struct PluginResult {
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Serialize)]
struct CallCommand<'a> {
    #[serde(rename = "type")]
    msg_type: &'a str,
    function: &'a str,
    context: CallContext<'a>,
}

#[derive(Serialize)]
struct CallContext<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<&'a str>,
}

// ── Plugin type enums ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    Exe,
    Python,
    #[serde(rename = "csharp")]
    CSharp,
    #[serde(rename = "powershell")]
    PowerShell,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginMode {
    Daemon,
    Oneshot,
}

/// How a plugin call should be dispatched (see PluginManager::resolve_call).
pub enum CallTarget {
    Oneshot { plugin_type: PluginType },
    Daemon,
}

// ── Data types for frontend ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPlugin {
    pub path: String,
    pub valid: bool,
    pub name: String,
    pub version: String,
    pub description: String,
    pub instruction: String,
    pub settings_description: String,
    pub settings_format: String,
    pub functions: Vec<PluginFunction>,
    pub error: String,
    pub plugin_type: PluginType,
    pub mode: PluginMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    pub path: String,
    pub enabled: bool,
    pub key_bindings: HashMap<String, String>,
    #[serde(default)]
    pub settings: String,
}

// ── Plugin process handle ───────────────────────────────────────

struct PluginProcess {
    child: Child,
    stdin: ChildStdin,
    /// Option so the reader can be moved into a watchdog thread during a call
    /// and restored afterwards (see call_function's timeout handling).
    reader: Option<BufReader<ChildStdout>>,
}

/// Metadata for oneshot scripts (no running process).
#[derive(Debug, Clone)]
struct OneshotMeta {
    plugin_type: PluginType,
}

// ── Plugin Manager ──────────────────────────────────────────────

pub struct PluginManager {
    processes: HashMap<String, PluginProcess>,
    /// Oneshot scripts: key = path, no background process.
    oneshot_scripts: HashMap<String, OneshotMeta>,
    hellos: HashMap<String, PluginHello>,
    /// key (uppercase) → (plugin_path, function_id)
    key_map: HashMap<String, (String, String)>,
    /// Windows Job Object — all child processes are assigned to this job.
    /// When ClipToAll exits (even if crashed/killed), Windows auto-kills all children.
    #[cfg(windows)]
    job_handle: Option<SendHandle>,
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

impl PluginManager {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
            oneshot_scripts: HashMap::new(),
            hellos: HashMap::new(),
            key_map: HashMap::new(),
            #[cfg(windows)]
            job_handle: Self::create_job_object(),
        }
    }

    /// Create a Windows Job Object with KILL_ON_JOB_CLOSE flag.
    /// When the last handle to this job is closed (i.e. ClipToAll exits/crashes),
    /// Windows automatically terminates all processes assigned to the job.
    #[cfg(windows)]
    fn create_job_object() -> Option<SendHandle> {
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok.is_err() {
                crate::log("plugins: failed to configure Job Object");
                let _ = windows::Win32::Foundation::CloseHandle(job);
                return None;
            }
            crate::log("plugins: Job Object created — child processes will be auto-killed on exit");
            Some(SendHandle(job))
        }
    }

    /// Get the plugins/ directory path (next to the exe). Creates it if missing.
    pub fn plugins_dir() -> Option<PathBuf> {
        let exe_dir = std::env::current_exe().ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;
        let dir = exe_dir.join("plugins");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }

    /// Find plugin exe files in the plugins/ subfolder.
    pub fn discover_exe_files() -> Vec<PathBuf> {
        let plugins_dir = match Self::plugins_dir() {
            Some(d) => d,
            None => return vec![],
        };
        let self_name = std::env::current_exe().ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));

        let mut result = vec![];
        if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("exe") {
                    continue;
                }
                let name = path.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if Some(&name) == self_name.as_ref() {
                    continue;
                }
                result.push(path);
            }
        }
        result
    }

    /// Probe an exe plugin: start it, read hello, stop it. Returns info or error.
    pub fn probe_plugin(path: &str) -> DiscoveredPlugin {
        match Self::spawn_and_hello_ext(path, PluginType::Exe) {
            Ok((mut child, mut stdin, _reader, hello)) => {
                // Send shutdown via our handle
                let _ = writeln!(stdin, r#"{{"type":"shutdown"}}"#);
                let _ = stdin.flush();
                drop(stdin);
                // Bounded wait: an exe that ignores shutdown must not hang the
                // Plugins-tab discovery. Give it 2s, then kill.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        _ => { let _ = child.kill(); let _ = child.wait(); break; }
                    }
                }
                DiscoveredPlugin {
                    path: path.into(),
                    valid: true,
                    name: hello.name,
                    version: hello.version,
                    description: hello.description,
                    instruction: hello.instruction,
                    settings_description: hello.settings_description,
                    settings_format: hello.settings_format,
                    functions: hello.functions,
                    error: String::new(),
                    plugin_type: PluginType::Exe,
                    mode: PluginMode::Daemon,
                }
            }
            Err(e) => {
                DiscoveredPlugin {
                    path: path.into(),
                    valid: false,
                    name: String::new(),
                    version: String::new(),
                    description: String::new(),
                    instruction: String::new(),
                    settings_description: String::new(),
                    settings_format: String::new(),
                    functions: vec![],
                    error: e,
                    plugin_type: PluginType::Exe,
                    mode: PluginMode::Daemon,
                }
            }
        }
    }

    /// Start an exe plugin in daemon mode and keep it running.
    pub fn start_plugin(&mut self, path: &str, key_bindings: &HashMap<String, String>) -> Result<PluginHello, String> {
        self.start_daemon(path, PluginType::Exe, key_bindings)
    }

    /// Start a plugin with explicit type and mode.
    pub fn start_plugin_ext(
        &mut self,
        path: &str,
        plugin_type: PluginType,
        mode: PluginMode,
        hello: &PluginHello,
        key_bindings: &HashMap<String, String>,
    ) -> Result<PluginHello, String> {
        match mode {
            PluginMode::Oneshot => {
                self.register_oneshot(path, plugin_type, hello.clone(), key_bindings);
                Ok(hello.clone())
            }
            PluginMode::Daemon => {
                self.start_daemon(path, plugin_type, key_bindings)
            }
        }
    }

    /// Start a daemon-mode plugin (exe or script).
    fn start_daemon(&mut self, path: &str, plugin_type: PluginType, key_bindings: &HashMap<String, String>) -> Result<PluginHello, String> {
        // Already running?
        if self.processes.contains_key(path) {
            if let Some(hello) = self.hellos.get(path) {
                return Ok(hello.clone());
            }
        }

        let (child, stdin, reader, hello) = Self::spawn_and_hello_ext(path, plugin_type)?;

        // Assign child process to Job Object so it gets killed if ClipToAll crashes
        #[cfg(windows)]
        self.assign_to_job(&child);

        self.register_key_bindings(path, &hello.functions, key_bindings);
        self.hellos.insert(path.to_string(), hello.clone());
        self.processes.insert(path.to_string(), PluginProcess { child, stdin, reader: Some(reader) });

        crate::log(&format!("plugins: started daemon \"{}\" ({})", hello.name, path));
        Ok(hello)
    }

    /// Assign a child process to the Job Object.
    #[cfg(windows)]
    fn assign_to_job(&self, child: &Child) {
        use std::os::windows::io::AsRawHandle;
        if let Some(SendHandle(job)) = self.job_handle {
            let proc_handle = HANDLE(child.as_raw_handle());
            unsafe {
                if let Err(e) = AssignProcessToJobObject(job, proc_handle) {
                    crate::log(&format!("plugins: failed to assign process to Job Object: {}", e));
                }
            }
        }
    }

    /// Register a oneshot script plugin (no background process).
    fn register_oneshot(
        &mut self,
        path: &str,
        plugin_type: PluginType,
        hello: PluginHello,
        key_bindings: &HashMap<String, String>,
    ) {
        self.register_key_bindings(path, &hello.functions, key_bindings);
        self.hellos.insert(path.to_string(), hello.clone());
        self.oneshot_scripts.insert(path.to_string(), OneshotMeta { plugin_type });
        crate::log(&format!("plugins: registered oneshot \"{}\" ({})", hello.name, path));
    }

    /// Register key bindings for a plugin's functions.
    fn register_key_bindings(&mut self, path: &str, functions: &[PluginFunction], key_bindings: &HashMap<String, String>) {
        for func in functions {
            let key = key_bindings
                .get(&func.id)
                .cloned()
                .unwrap_or_else(|| func.default_key.clone())
                .to_uppercase();
            use std::collections::hash_map::Entry;
            match self.key_map.entry(key.clone()) {
                Entry::Vacant(e) => { e.insert((path.to_string(), func.id.clone())); }
                Entry::Occupied(_) => {
                    crate::log(&format!("plugins: key '{}' for {}/{} skipped — already taken", key, path, func.id));
                }
            }
        }
    }

    /// Build the command to run a plugin/script.
    fn build_command(path: &str, plugin_type: PluginType) -> Command {
        // PowerShell: resolve the built-in system binary so a rogue
        // powershell.exe earlier on PATH can't shadow it. python/dotnet are
        // deliberately left on PATH (no single canonical install path; same-user
        // trust boundary).
        match plugin_type {
            PluginType::Python => {
                let mut cmd = Command::new("python");
                cmd.arg(path);
                cmd
            }
            PluginType::CSharp => {
                let mut cmd = Command::new("dotnet");
                cmd.arg("run").arg(path).arg("--property:WarningLevel=0");
                cmd
            }
            PluginType::PowerShell => {
                let mut cmd = Command::new(powershell_path());
                cmd.args(["-NoProfile", "-File"]).arg(path);
                cmd
            }
            PluginType::Exe => {
                Command::new(path)
            }
        }
    }

    /// Spawn a plugin process with --daemon flag, read hello with 20s timeout.
    fn spawn_and_hello_ext(path: &str, plugin_type: PluginType) -> Result<(Child, ChildStdin, BufReader<ChildStdout>, PluginHello), String> {
        let mut cmd = Self::build_command(path, plugin_type);
        // For C#, script args go after "--" separator
        if plugin_type == PluginType::CSharp {
            cmd.args(["--", "--daemon"]);
        } else {
            cmd.arg("--daemon");
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn()
            .map_err(|e| format!("spawn failed: {}", e))?;

        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        // Read hello with timeout
        let (tx, rx) = std::sync::mpsc::channel::<Result<(String, BufReader<ChildStdout>), String>>();
        let mut reader = BufReader::new(stdout);

        std::thread::spawn(move || {
            let mut line = String::new();
            let result = match reader.read_line(&mut line) {
                Ok(0) => Err("EOF before hello".into()),
                Ok(_) => Ok((line, reader)),
                Err(e) => Err(format!("read: {}", e)),
            };
            let _ = tx.send(result);
        });

        match rx.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(Ok((line, reader))) => {
                let hello: PluginHello = serde_json::from_str(line.trim())
                    .map_err(|e| format!("bad hello JSON: {} — raw: {}", e, line.trim()))?;
                Ok((child, stdin, reader, hello))
            }
            Ok(Err(e)) => {
                let _ = child.kill();
                Err(e)
            }
            Err(_) => {
                let _ = child.kill();
                Err("timeout: no hello within 20s".into())
            }
        }
    }

    /// Stop a plugin (daemon or oneshot).
    pub fn stop_plugin(&mut self, path: &str) {
        self.key_map.retain(|_, (p, _)| p != path);
        self.hellos.remove(path);
        self.oneshot_scripts.remove(path);

        if let Some(mut proc) = self.processes.remove(path) {
            let _ = writeln!(proc.stdin, r#"{{"type":"shutdown"}}"#);
            let _ = proc.stdin.flush();
            crate::log(&format!("plugins: stopping {}", path));
            // Give 2s to exit, then kill
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let _ = proc.child.kill();
            });
        }
    }

    /// Stop all plugins.
    pub fn stop_all(&mut self) {
        let paths: Vec<String> = self.processes.keys()
            .chain(self.oneshot_scripts.keys())
            .cloned()
            .collect();
        for path in paths {
            self.stop_plugin(&path);
        }
    }

    /// Get the key→(plugin_path, function_id) map for overlay dispatch.
    pub fn get_key_map(&self) -> HashMap<String, (String, String)> {
        self.key_map.clone()
    }

    /// Resolve how a plugin call should be dispatched, using only a cheap map
    /// lookup. The caller uses this to decide whether to run the call WITHOUT
    /// holding the manager mutex (oneshot) — so a hung script can never wedge
    /// the mutex that the hotkey / Plugins tab / Exit all depend on.
    pub fn resolve_call(&self, path: &str) -> Option<CallTarget> {
        if let Some(meta) = self.oneshot_scripts.get(path) {
            Some(CallTarget::Oneshot { plugin_type: meta.plugin_type })
        } else if self.processes.contains_key(path) {
            Some(CallTarget::Daemon)
        } else {
            None
        }
    }

    /// Call a running DAEMON plugin. Must be dispatched via resolve_call.
    pub fn call_function_daemon(&mut self, path: &str, function_id: &str, settings: Option<&str>) -> Result<PluginResult, String> {
        // Daemon plugin — write the request, then read the response with a
        // watchdog timeout. A hung plugin must not freeze the capture thread
        // (which holds the manager mutex) forever (BUGS#9).
        let cmd = CallCommand {
            msg_type: "call",
            function: function_id,
            context: CallContext { settings },
        };
        let json = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        crate::log(&format!("plugins: calling {} → {} (settings={})",
            path, function_id, if settings.is_some() { "set" } else { "none" }));

        // Write the request and take the reader out so it can be read on a
        // separate thread (bounded by recv_timeout below).
        let reader = {
            let proc = self.processes.get_mut(path)
                .ok_or_else(|| format!("plugin not running: {}", path))?;
            writeln!(proc.stdin, "{}", json).map_err(|e| format!("write: {}", e))?;
            proc.stdin.flush().map_err(|e| format!("flush: {}", e))?;
            proc.reader.take()
                .ok_or_else(|| "plugin busy (a previous call is still in progress)".to_string())?
        };

        let (tx, rx) = std::sync::mpsc::channel::<(BufReader<ChildStdout>, std::io::Result<String>)>();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut line = String::new();
            let res = reader.read_line(&mut line).map(|_| line);
            let _ = tx.send((reader, res));
        });

        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok((reader, Ok(line))) => {
                if let Some(proc) = self.processes.get_mut(path) {
                    proc.reader = Some(reader);
                }
                crate::log(&format!("plugins: response from {}: {}", path, line.trim()));
                serde_json::from_str(line.trim()).map_err(|e| format!("bad result: {}", e))
            }
            Ok((reader, Err(e))) => {
                if let Some(proc) = self.processes.get_mut(path) {
                    proc.reader = Some(reader);
                }
                Err(format!("read: {}", e))
            }
            Err(_) => {
                // Timed out: the plugin's stdout is now out of sync — kill it and
                // drop its registration so it can't wedge future calls.
                crate::log(&format!("plugins: call to {} timed out after 10s; killing plugin", path));
                if let Some(mut proc) = self.processes.remove(path) {
                    let _ = proc.child.kill();
                }
                self.key_map.retain(|_, (p, _)| p != path);
                self.hellos.remove(path);
                Err("plugin call timed out".into())
            }
        }
    }

    /// Run a oneshot script: spawn, pass --call, capture stdout, return result.
    /// Public and self-contained (no &self) so the caller can run it WITHOUT the
    /// manager mutex. Bounded by a 30s timeout so a hung script self-terminates
    /// instead of blocking forever (BUGS#9 / oneshot half).
    pub fn run_oneshot(
        path: &str, plugin_type: PluginType, function_id: &str, settings: Option<&str>,
    ) -> Result<PluginResult, String> {
        let call_json = serde_json::json!({
            "type": "call",
            "function": function_id,
            "context": { "settings": settings.unwrap_or("") }
        })
        .to_string();

        // Write the call context (which may include SECRET settings, e.g. a
        // plugin password) to a temp file and pass it as `--call @<file>` instead
        // of inline on the command line. A process command line is readable by any
        // of the user's other processes (Win32_Process.CommandLine / Process
        // Explorer), which would defeat the on-disk DPAPI encryption (3.7). The
        // file lives in the user's private temp dir and is deleted the moment the
        // call returns (RAII guard), on every exit path including timeout.
        struct TempFileGuard(PathBuf);
        impl Drop for TempFileGuard {
            fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
        }
        let call_file = std::env::temp_dir()
            .join(format!("cta_call_{}.json", uuid::Uuid::new_v4()));
        // Arm the delete guard BEFORE writing, so even a partially-written file
        // (e.g. disk full) can't be left behind holding secret bytes.
        let _call_guard = TempFileGuard(call_file.clone());
        std::fs::write(&call_file, &call_json)
            .map_err(|e| format!("Failed to write call file: {}", e))?;
        let call_arg = format!("@{}", call_file.display());

        let mut cmd = Self::build_command(path, plugin_type);
        if plugin_type == PluginType::CSharp {
            cmd.args(["--", "--call", &call_arg]);
        } else {
            cmd.args(["--call", &call_arg]);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        crate::log(&format!("plugins: oneshot call {} → {}", path, function_id));
        let mut child = cmd.spawn().map_err(|e| format!("Failed to run script: {}", e))?;

        // Poll for exit with a hard deadline; kill if the script hangs. (Oneshot
        // plugins print a small JSON result, so not draining stdout during the
        // wait is fine; a script emitting >64KB before exit could still block —
        // acceptable for the plugin contract.)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match child.try_wait().map_err(|e| format!("wait: {}", e))? {
                Some(_) => break,
                None => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("oneshot plugin '{}' timed out after 30s", path));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }

        let output = child.wait_with_output().map_err(|e| format!("output: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        crate::log(&format!("plugins: oneshot output from {}: {}", path, stdout));

        // Try to parse last non-empty line as PluginResult JSON
        if let Some(last_line) = stdout.lines().rev().find(|l| !l.trim().is_empty()) {
            if let Ok(result) = serde_json::from_str::<PluginResult>(last_line.trim()) {
                return Ok(result);
            }
        }

        // Fallback: treat raw stdout as message
        Ok(PluginResult {
            status: if output.status.success() { "ok".to_string() } else { "error".to_string() },
            message: Some(if stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            } else {
                stdout
            }),
            action: None,
        })
    }
}

#[cfg(windows)]
impl Drop for PluginManager {
    fn drop(&mut self) {
        if let Some(SendHandle(job)) = self.job_handle.take() {
            unsafe { let _ = windows::Win32::Foundation::CloseHandle(job); }
        }
    }
}

// ── Script metadata parsing ─────────────────────────────────────

/// Parse metadata from script comment headers.
/// Python uses `# @key: value`, C# uses `// @key: value`.
/// Returns (PluginHello, PluginMode) or None if `@plugin:` is missing.
pub fn parse_script_metadata(content: &str, plugin_type: PluginType) -> Option<(PluginHello, PluginMode)> {
    let prefix = match plugin_type {
        PluginType::Python | PluginType::PowerShell => "# @",
        PluginType::CSharp => "// @",
        PluginType::Exe => return None,
    };

    let mut name = String::new();
    let mut description = String::new();
    let mut version = "1.0.0".to_string();
    let mut mode = PluginMode::Oneshot;
    let mut key = String::new();
    let mut function_label = String::new();
    let mut instruction = String::new();
    let mut settings_description = String::new();
    let mut settings_format = String::new();
    let mut explicit_functions: Vec<PluginFunction> = vec![];

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(after) = trimmed.strip_prefix(prefix) {
            if let Some((tag, value)) = after.split_once(':') {
                let tag = tag.trim().to_lowercase();
                let value = value.trim().to_string();
                match tag.as_str() {
                    "plugin" => name = value,
                    "description" => description = value,
                    "version" => version = value,
                    "mode" => {
                        mode = if value.to_lowercase() == "daemon" {
                            PluginMode::Daemon
                        } else {
                            PluginMode::Oneshot
                        };
                    }
                    "key" => key = value.to_uppercase(),
                    "label" => function_label = value,
                    "instruction" => instruction = value,
                    "settings_description" => settings_description = value,
                    "settings_format" => settings_format = value,
                    "function" => {
                        // Format: @function: id, label, key
                        let parts: Vec<&str> = value.splitn(3, ',').collect();
                        if parts.len() >= 2 {
                            let fid = parts[0].trim().to_string();
                            let flabel = parts[1].trim().to_string();
                            if !fid.is_empty() && !flabel.is_empty() {
                                let fkey = if parts.len() >= 3 {
                                    parts[2].trim().to_uppercase()
                                } else {
                                    "R".to_string()
                                };
                                explicit_functions.push(PluginFunction {
                                    id: fid,
                                    label: flabel,
                                    default_key: fkey,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("//")
        {
            // Stop parsing at first non-comment, non-empty line
            break;
        }
    }

    if name.is_empty() {
        return None;
    }

    // Use explicit @function: tags if present, otherwise fall back to @key/@label.
    // If neither is specified AND mode is daemon, allow zero functions (background-only plugin).
    let functions = if !explicit_functions.is_empty() {
        explicit_functions
    } else if key.is_empty() && function_label.is_empty() && mode == PluginMode::Daemon {
        // Daemon with no declared functions — runs in background only, no hotkeys needed
        vec![]
    } else {
        let func_id = name.to_lowercase().replace([' ', '-'], "_");
        let func_label = if function_label.is_empty() {
            format!("Run {}", name)
        } else {
            function_label
        };
        let default_key = if key.is_empty() { "R".to_string() } else { key };
        vec![PluginFunction { id: func_id, label: func_label, default_key }]
    };

    Some((
        PluginHello {
            name,
            version,
            description,
            instruction,
            settings_description,
            settings_format,
            functions,
        },
        mode,
    ))
}

/// Discover .py, .cs, and .ps1 script files in the plugins/ directory.
pub fn discover_script_files() -> Vec<DiscoveredPlugin> {
    let dir = match PluginManager::plugins_dir() {
        Some(d) => d,
        None => return vec![],
    };

    let mut result = vec![];
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let plugin_type = match ext {
            "py" => PluginType::Python,
            "cs" => PluginType::CSharp,
            "ps1" => PluginType::PowerShell,
            _ => continue,
        };

        let path_str = path.to_string_lossy().to_string();

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                result.push(DiscoveredPlugin {
                    path: path_str,
                    valid: false,
                    name: String::new(),
                    version: String::new(),
                    description: String::new(),
                    instruction: String::new(),
                    settings_description: String::new(),
                    settings_format: String::new(),
                    functions: vec![],
                    error: format!("Cannot read file: {}", e),
                    plugin_type,
                    mode: PluginMode::Oneshot,
                });
                continue;
            }
        };

        match parse_script_metadata(&content, plugin_type) {
            Some((hello, mode)) => {
                result.push(DiscoveredPlugin {
                    path: path_str,
                    valid: true,
                    name: hello.name,
                    version: hello.version,
                    description: hello.description,
                    instruction: hello.instruction,
                    settings_description: hello.settings_description,
                    settings_format: hello.settings_format,
                    functions: hello.functions,
                    error: String::new(),
                    plugin_type,
                    mode,
                });
            }
            None => {
                result.push(DiscoveredPlugin {
                    path: path_str,
                    valid: false,
                    name: String::new(),
                    version: String::new(),
                    description: String::new(),
                    instruction: String::new(),
                    settings_description: String::new(),
                    settings_format: String::new(),
                    functions: vec![],
                    error: format!("Missing {} header", match plugin_type {
                        PluginType::Python | PluginType::PowerShell => "# @plugin:",
                        PluginType::CSharp => "// @plugin:",
                        _ => "@plugin:",
                    }),
                    plugin_type,
                    mode: PluginMode::Oneshot,
                });
            }
        }
    }
    // Sort by file modification time (oldest first = chronological "date added" order)
    result.sort_by(|a, b| {
        let mtime_a = std::fs::metadata(&a.path).and_then(|m| m.modified()).ok();
        let mtime_b = std::fs::metadata(&b.path).and_then(|m| m.modified()).ok();
        mtime_a.cmp(&mtime_b)
    });
    result
}

/// Absolute path to built-in Windows PowerShell, so a rogue powershell.exe
/// earlier on PATH can't be picked up. Falls back to bare name if missing.
pub(crate) fn powershell_path() -> std::path::PathBuf {
    if let Ok(root) = std::env::var("SystemRoot") {
        let p = std::path::PathBuf::from(root)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if p.exists() {
            return p;
        }
    }
    std::path::PathBuf::from("powershell")
}

/// Check if a runtime (python/dotnet) is available in PATH.
pub fn check_runtime(language: &str) -> Result<String, String> {
    let cmd_name = match language {
        "python" => "python",
        "csharp" => "dotnet",
        "powershell" => "powershell",
        _ => return Err(format!("Unknown language: {}", language)),
    };

    // PowerShell: resolve the built-in system binary so a rogue powershell.exe
    // earlier on PATH can't shadow it. python/dotnet deliberately stay on PATH
    // (no single canonical install path; same-user trust boundary).
    let mut cmd = if language == "powershell" {
        Command::new(powershell_path())
    } else {
        Command::new(cmd_name)
    };
    if language == "powershell" {
        cmd.args(["-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"]);
    } else {
        cmd.arg("--version");
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output()
        .map_err(|e| format!("{} not found in PATH: {}", cmd_name, e))?;

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("{} returned no output", cmd_name))
        } else {
            Ok(stderr)
        }
    } else {
        Ok(version)
    }
}

/// Detect plugin type and mode from file extension.
/// For scripts, reads metadata to determine mode.
pub fn detect_plugin_type(path: &str) -> (PluginType, PluginMode) {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "py" => {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((_, mode)) = parse_script_metadata(&content, PluginType::Python) {
                    return (PluginType::Python, mode);
                }
            }
            (PluginType::Python, PluginMode::Oneshot)
        }
        "cs" => {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((_, mode)) = parse_script_metadata(&content, PluginType::CSharp) {
                    return (PluginType::CSharp, mode);
                }
            }
            (PluginType::CSharp, PluginMode::Oneshot)
        }
        "ps1" => {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((_, mode)) = parse_script_metadata(&content, PluginType::PowerShell) {
                    return (PluginType::PowerShell, mode);
                }
            }
            (PluginType::PowerShell, PluginMode::Oneshot)
        }
        _ => (PluginType::Exe, PluginMode::Daemon),
    }
}

/// Thread-safe wrapper for Tauri state.
pub struct PluginManagerState(pub Mutex<PluginManager>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_python_oneshot() {
        let src = "# @plugin: Test\n# @key: E\n# @label: Do it\nprint('x')";
        let (hello, mode) = parse_script_metadata(src, PluginType::Python).unwrap();
        assert_eq!(hello.name, "Test");
        assert_eq!(mode, PluginMode::Oneshot);
        assert_eq!(hello.functions.len(), 1);
        assert_eq!(hello.functions[0].default_key, "E");
    }

    #[test]
    fn parse_multi_function() {
        let src = "# @plugin: Enc\n# @function: encrypt, Encrypt, E\n# @function: decrypt, Decrypt, D\n";
        let (hello, _) = parse_script_metadata(src, PluginType::Python).unwrap();
        assert_eq!(hello.functions.len(), 2);
        assert_eq!(hello.functions[1].id, "decrypt");
    }

    #[test]
    fn parse_missing_plugin_tag_is_none() {
        assert!(parse_script_metadata("print('x')", PluginType::Python).is_none());
    }

    #[test]
    fn csharp_uses_slash_prefix() {
        let src = "// @plugin: CsPlug\n// @mode: daemon\n";
        let (hello, mode) = parse_script_metadata(src, PluginType::CSharp).unwrap();
        assert_eq!(hello.name, "CsPlug");
        assert_eq!(mode, PluginMode::Daemon);
    }
}
