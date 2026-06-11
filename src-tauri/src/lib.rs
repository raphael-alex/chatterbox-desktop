use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use tauri::State;

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

struct AppState {
    python_child: Mutex<Option<Child>>,
    next_id: Mutex<i64>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            python_child: Mutex::new(None),
            next_id: Mutex::new(1),
        }
    }
}

// ─── Python IPC ───────────────────────────────────────────────────────────────

async fn send_to_python(state: &State<'_, AppState>, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    println!("[Rust] send_to_python: method={}", method);
    let mut child_guard = state.python_child.lock().map_err(|e| e.to_string())?;
    let child = child_guard.as_mut().ok_or("Python process not running")?;

    let id = {
        let mut counter = state.next_id.lock().map_err(|e| e.to_string())?;
        let id = serde_json::json!(*counter);
        *counter += 1;
        id
    };

    let msg = JsonRpcMessage {
        method: method.to_string(),
        params,
        id: id.clone(),
    };

    let line = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    println!("[Rust] -> Python: {}", line);
    let stdin = child.stdin.as_mut().ok_or("No stdin")?;
    stdin.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    stdin.write_all(b"\n").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;

    let stdout = child.stdout.as_mut().ok_or("No stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut response_line = String::new();
    reader.read_line(&mut response_line).map_err(|e| e.to_string())?;
    println!("[Rust] <- Python: {}", response_line.trim());

    let resp: JsonRpcResponse = serde_json::from_str(&response_line)
        .map_err(|e| format!("Failed to parse response: {} | line: {}", e, response_line))?;

    if let Some(err) = resp.error {
        return Err(format!("Python error: {}", err));
    }

    resp.result.ok_or_else(|| "No result".to_string())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn find_python() -> String {
    std::env::var("CHATTERBOX_PYTHON").unwrap_or_else(|_| {
        let candidates = [
            "/Users/kaku/.pyenv/shims/python3",
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
        ];
        for path in &candidates {
            if std::process::Command::new(path)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return path.to_string();
            }
        }
        "python3".to_string()
    })
}

// ─── Tauri Commands ────────────────────────────────────────────────────────────

#[tauri::command]
async fn start_session(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    println!("[Rust] start_session called");
    // Check if already running
    {
        let guard = state.python_child.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            println!("[Rust] Session already running");
            return Err("Session already running".to_string());
        }
    }

    // Find Python interpreter - prefer pyenv or CHATTERBOX_PYTHON env var
    let python_path = find_python();

    // Find Python script path
    let script_path = {
        let raw_path = if cfg!(debug_assertions) {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("python_ipc_server.py")
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.join("python_ipc_server.py")))
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("..")
                        .join("python_ipc_server.py")
                })
        };
        raw_path.canonicalize().unwrap_or(raw_path)
    };
    println!("[Rust] Python path: {}", python_path);
    println!("[Rust] Script path: {}", script_path.display());

    // The chatterbox project is at the same level as chatterbox-desktop
    let chatterbox_dir = script_path
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("chatterbox"))
        .unwrap_or_else(|| std::path::PathBuf::from("/Users/kaku/MakeMoney/chatterbox"));
    println!("[Rust] Chatterbox dir: {}", chatterbox_dir.display());

    // Spawn Python process with CWD set to chatterbox project root
    // so config.yaml is resolved correctly (config.py uses Path("config.yaml"))
    let child = std::process::Command::new(&python_path)
        .arg(&script_path)
        .current_dir(&chatterbox_dir)
        .envs(std::env::vars())
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start Python: {}. Is python3 in PATH?", e))?;
    println!("[Rust] Python process spawned");

    // Store child process
    {
        let mut guard = state.python_child.lock().map_err(|e| e.to_string())?;
        *guard = Some(child);
    }

    // Initialize session
    println!("[Rust] Sending start_session to Python...");
    let result = send_to_python(&state, "start_session", serde_json::json!({})).await;
    println!("[Rust] Python result: {:?}", result);

    result
}

#[tauri::command]
async fn send_message(state: State<'_, AppState>, text: String) -> Result<serde_json::Value, String> {
    send_to_python(&state, "add_message", serde_json::json!({ "text": text })).await
}

#[tauri::command]
async fn quit(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let result = send_to_python(&state, "quit", serde_json::json!({})).await;

    // Kill Python process
    if let Ok(mut guard) = state.python_child.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    result
}

#[tauri::command]
async fn check_devices() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "status": "ok" }))
}

// ─── Main ──────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            start_session,
            send_message,
            quit,
            check_devices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
