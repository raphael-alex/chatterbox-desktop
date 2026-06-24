use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::{Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};

// ─── IPC Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcMessage {
    #[serde(rename = "method")]
    method: String,
    #[serde(rename = "params")]
    params: serde_json::Value,
    #[serde(rename = "id")]
    id: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcResponse {
    #[serde(rename = "id")]
    id: serde_json::Value,
    #[serde(rename = "result")]
    result: Option<serde_json::Value>,
    #[serde(rename = "error")]
    error: Option<serde_json::Value>,
}

// ─── App State ────────────────────────────────────────────────────────────────

type ResponseSender = oneshot::Sender<Result<serde_json::Value, String>>;

struct PythonSession {
    _child: Child,
    stdin: tokio::process::ChildStdin,
    pending: Arc<Mutex<HashMap<i64, ResponseSender>>>,
}

struct AppState {
    session: Mutex<Option<PythonSession>>,
    next_id: AtomicI64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            next_id: AtomicI64::new(1),
        }
    }
}

/// Send a request to Python and wait for the response (with 60s timeout)
async fn send_ipc(state: &AppState, method: &str, params: serde_json::Value, id: i64) -> Result<serde_json::Value, String> {
    let (tx, rx) = oneshot::channel();

    // Register pending response
    {
        let guard = state.session.lock().await;
        let session = guard.as_ref().ok_or("No active session")?;
        session.pending.lock().await.insert(id, tx);
    }

    // Build and send message
    let msg = JsonRpcMessage {
        method: method.to_string(),
        params,
        id: serde_json::json!(id),
    };
    let line = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    println!("[Rust] -> Python: {}", line);

    {
        let mut guard = state.session.lock().await;
        let session = guard.as_mut().ok_or("No active session")?;
        session.stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
        session.stdin.write_all(b"\n").await.map_err(|e| e.to_string())?;
        session.stdin.flush().await.map_err(|e| e.to_string())?;
    }

    // Wait for response with timeout
    match tokio::time::timeout(std::time::Duration::from_secs(60), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => {
            let mut guard = state.session.lock().await;
            if let Some(s) = guard.as_mut() {
                s.pending.lock().await.remove(&id);
            }
            Err("Response channel dropped".to_string())
        }
        Err(_) => {
            let mut guard = state.session.lock().await;
            if let Some(s) = guard.as_mut() {
                s.pending.lock().await.remove(&id);
            }
            Err("Request timed out (60s)".to_string())
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn find_python() -> String {
    // 1. Check CHATTERBOX_PYTHON env var
    if let Ok(path) = std::env::var("CHATTERBOX_PYTHON") {
        if is_valid_python(&path) {
            println!("[Rust] Python found via CHATTERBOX_PYTHON: {}", path);
            return path;
        }
    }

    // 2. Try pyenv shims (common user-managed Python)
    let home = std::env::var("HOME").unwrap_or_default();
    let pyenv_paths = [
        format!("{}/.pyenv/shims/python3", home),
        format!("{}/.pyenv/versions/*/bin/python3", home),
    ];
    for pattern in &pyenv_paths {
        if pattern.contains('*') {
            // Expand glob
            if let Ok(entries) = std::fs::read_dir(std::path::Path::new(&pattern[..pattern.rfind('/').unwrap()])) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() && p.to_string_lossy().ends_with("python3") {
                        let ps = p.to_string_lossy().to_string();
                        if is_valid_python(&ps) {
                            println!("[Rust] Python found via pyenv: {}", ps);
                            return ps;
                        }
                    }
                }
            }
        } else if is_valid_python(pattern) {
            println!("[Rust] Python found via pyenv: {}", pattern);
            return pattern.clone();
        }
    }

    // 3. Try `which python3` / `which python`
    for cmd in &["python3", "python"] {
        if let Ok(output) = std::process::Command::new("which").arg(cmd).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if is_valid_python(&path) {
                    println!("[Rust] Python found via which: {}", path);
                    return path;
                }
            }
        }
    }

    // 4. Try common system paths
    let candidates = [
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
        "/usr/bin/python3",
        "/usr/bin/python",
    ];
    for path in &candidates {
        if is_valid_python(path) {
            println!("[Rust] Python found at: {}", path);
            return path.to_string();
        }
    }

    // 5. Try PATH lookup
    for cmd in &["python3", "python"] {
        if is_valid_python(cmd) {
            println!("[Rust] Python found via PATH: {}", cmd);
            return cmd.to_string();
        }
    }

    "python3".to_string()
}

fn is_valid_python(path: &str) -> bool {
    std::process::Command::new(path).arg("--version").output()
        .map(|o| o.status.success()).unwrap_or(false)
}

fn augment_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let pyenv_shims = format!("{}/.pyenv/shims", home);
    let extra = [
        pyenv_shims.as_str(),
        "/opt/homebrew/bin", "/opt/homebrew/sbin",
        "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin",
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    let mut paths: Vec<&str> = extra.iter().copied().collect();
    paths.push(&current);
    paths.join(":")
}

fn load_env_file() -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let env_paths = if cfg!(debug_assertions) {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        vec![manifest.join("..").join(".env")]
    } else {
        let mut paths = vec![];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                paths.push(d.join(".env"));
                paths.push(d.join("../Resources/.env"));
            }
        }
        paths
    };
    for path in &env_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            println!("[Rust] Loading env from: {}", path.display());
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim().to_string();
                    let value = value.trim().to_string();
                    if std::env::var(&key).is_err() { vars.insert(key, value); }
                }
            }
            break;
        }
    }
    vars
}

/// Spawn background task to read Python stdout and route responses
fn spawn_reader(
    mut reader: tokio::io::BufReader<tokio::process::ChildStdout>,
    pending: Arc<Mutex<HashMap<i64, ResponseSender>>>,
) {
    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    println!("[Rust] Python stdout closed");
                    let mut p = pending.lock().await;
                    for (_, tx) in p.drain() { let _ = tx.send(Err("Python exited".into())); }
                    break;
                }
                Ok(_) => {
                    println!("[Rust] <- Python: {}", line.trim());
                    if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&line) {
                        let id = match &resp.id {
                            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                            _ => 0,
                        };
                        let result = if let Some(err) = resp.error {
                            Err(format!("Python error: {}", err))
                        } else {
                            resp.result.ok_or_else(|| "No result".into())
                        };
                        if let Some(tx) = pending.lock().await.remove(&id) {
                            let _ = tx.send(result);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Rust] Read error: {}", e);
                    break;
                }
            }
        }
    });
}

// ─── Tauri Commands ────────────────────────────────────────────────────────────

#[tauri::command]
async fn start_session(state: State<'_, AppState>, app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    println!("[Rust] start_session called");

    // Clean up old session
    {
        let mut guard = state.session.lock().await;
        *guard = None;
    }

    let python_path = find_python();
    let env_vars = load_env_file();
    let (script_path, chatterbox_dir) = if cfg!(debug_assertions) {
        let m = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        (m.join("..").join("python_ipc_server.py"), m.join("..").join("..").join("chatterbox"))
    } else {
        let rd = app_handle.path().resource_dir().map_err(|e| e.to_string())?;
        (rd.join("python_ipc_server.py"), rd.clone())
    };
    let script_path = script_path.canonicalize().unwrap_or(script_path);
    let chatterbox_dir = chatterbox_dir.canonicalize().unwrap_or(chatterbox_dir);
    println!("[Rust] Python: {}, Script: {}, CWD: {}", python_path, script_path.display(), chatterbox_dir.display());

    let mut child = Command::new(&python_path)
        .arg(&script_path).current_dir(&chatterbox_dir)
        .envs(std::env::vars()).envs(&env_vars)
        .env("PYTHONUNBUFFERED", "1").env("PYTHONIOENCODING", "utf-8")
        .env("PATH", augment_path())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start Python: {}", e))?;
    println!("[Rust] Python spawned");

    // Log stderr
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = r.read_line(&mut line).await {
                if n == 0 { break; }
                eprint!("[Python stderr] {}", line);
                line.clear();
            }
        });
    }

    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stdin = child.stdin.take().ok_or("No stdin")?;
    let pending: Arc<Mutex<HashMap<i64, ResponseSender>>> = Arc::new(Mutex::new(HashMap::new()));
    spawn_reader(BufReader::new(stdout), pending.clone());

    {
        let mut guard = state.session.lock().await;
        *guard = Some(PythonSession { _child: child, stdin, pending });
    }

    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    send_ipc(&state, "start_session", serde_json::json!({}), id).await
}

#[tauri::command]
async fn send_message(state: State<'_, AppState>, text: String) -> Result<serde_json::Value, String> {
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    send_ipc(&state, "add_message", serde_json::json!({ "text": text }), id).await
}

#[tauri::command]
async fn get_greeting(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    send_ipc(&state, "get_greeting", serde_json::json!({}), id).await
}

#[tauri::command]
async fn quit(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let result = send_ipc(&state, "quit", serde_json::json!({}), id).await;
    let mut guard = state.session.lock().await;
    if let Some(mut s) = guard.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let _ = s._child.kill().await;
            let _ = s._child.wait().await;
        }).await;
    }
    result
}

#[tauri::command]
async fn read_audio_file(path: String) -> Result<String, String> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("Read {}: {}", path, e))?;
    let b64 = BASE64.encode(&bytes);
    let _ = tokio::fs::remove_file(&path).await;
    Ok(b64)
}

#[tauri::command]
async fn transcribe_and_reply(state: State<'_, AppState>, audio_b64: String, mime_type: String) -> Result<serde_json::Value, String> {
    let audio_bytes = BASE64.decode(&audio_b64).map_err(|e| e.to_string())?;
    let mut tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut tmp, &audio_bytes).map_err(|e| e.to_string())?;
    std::io::Write::flush(&mut tmp).map_err(|e| e.to_string())?;
    let tmp_path = tmp.into_temp_path().keep().map_err(|e| e.to_string())?;
    let p = tmp_path.to_string_lossy().to_string();
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let result = send_ipc(&state, "transcribe_and_reply", serde_json::json!({ "audio_file": p, "mime_type": mime_type }), id).await;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            start_session, get_greeting, send_message, quit, read_audio_file, transcribe_and_reply,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
