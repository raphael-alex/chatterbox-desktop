#[allow(unused_imports)]
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

const MAX_MESSAGES: usize = 200;
const TRIM_TO: usize = 150;

#[wasm_bindgen(inline_js = "
export async function invokeSafe(cmd, args) {
    try {
        const result = await window.__TAURI__.core.invoke(cmd, args);
        return { ok: true, value: result };
    } catch (e) {
        return { ok: false, error: String(e) };
    }
}
")]
extern "C" {
    async fn invokeSafe(cmd: &str, args: JsValue) -> JsValue;
}

/// Invoke a Tauri command and return the unwrapped result value
async fn invoke_tauri(cmd: &str, args: JsValue) -> Result<serde_json::Value, String> {
    let result = invokeSafe(cmd, args).await;
    let wrapped: serde_json::Value = serde_wasm_bindgen::from_value(result)
        .map_err(|e| format!("Parse error: {}", e))?;
    if wrapped.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(wrapped.get("value").cloned().unwrap_or(serde_json::Value::Null))
    } else {
        Err(wrapped.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error").to_string())
    }
}

/// Trim messages if over limit
fn trim_messages(msgs: &mut Vec<Message>) {
    if msgs.len() > MAX_MESSAGES {
        let excess = msgs.len() - TRIM_TO;
        msgs.drain(0..excess);
    }
}

#[wasm_bindgen(inline_js = "
export function startRecording() {
    return new Promise((resolve, reject) => {
        navigator.mediaDevices.getUserMedia({ audio: true, video: false })
            .then(stream => {
                const recorder = new MediaRecorder(stream);
                const chunks = [];
                recorder.ondataavailable = (e) => {
                    if (e.data.size > 0) chunks.push(e.data);
                };
                recorder.start();
                resolve({
                    recorder: recorder,
                    stream: stream,
                    chunks: chunks,
                    mimeType: recorder.mimeType || 'audio/webm'
                });
            })
            .catch(err => reject(err));
    });
}
")]
extern "C" {
    fn startRecording() -> js_sys::Promise;
}

#[wasm_bindgen(inline_js = "
export function stopRecording(handle) {
    return new Promise((resolve) => {
        const recorder = handle.recorder;
        const chunks = handle.chunks;
        const mimeType = handle.mimeType;
        recorder.onstop = () => {
            const blob = new Blob(chunks, { type: mimeType });
            handle.stream.getTracks().forEach(t => t.stop());
            const reader = new FileReader();
            reader.onloadend = () => {
                const base64 = reader.result.split(',')[1];
                resolve({ base64: base64, mimeType: mimeType });
            };
            reader.readAsDataURL(blob);
        };
        recorder.stop();
    });
}
")]
extern "C" {
    fn stopRecording(handle: &JsValue) -> js_sys::Promise;
}

#[derive(Serialize, Deserialize)]
struct AudioArgs {
    #[serde(rename = "audioB64")]
    audio_b64: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

#[derive(Clone, Debug, PartialEq)]
enum MsgRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, PartialEq)]
struct Message {
    role: MsgRole,
    text: String,
}

#[component]
fn MessageList(messages: Vec<Message>) -> Element {
    rsx! {
        div { class: "messages",
            for msg in messages.iter() {
                div { class: format_args!("message {}", match msg.role {
                    MsgRole::User => "user",
                    MsgRole::Assistant => "assistant",
                    MsgRole::System => "system",
                }),
                    div { class: "bubble",
                        match msg.role {
                            MsgRole::Assistant => rsx! {
                                div { class: "name", "Luna" }
                                div { "{msg.text}" }
                            },
                            _ => rsx! {
                                div { "{msg.text}" }
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn LoadingIndicator() -> Element {
    rsx! {
        div { class: "message assistant",
            div { class: "bubble",
                div { class: "name", "Luna" }
                div { class: "loading",
                    div { class: "loading-dot" }
                    div { class: "loading-dot" }
                    div { class: "loading-dot" }
                }
            }
        }
    }
}

#[component]
fn TextInput(
    input_text: String,
    is_loading: bool,
    session_active: bool,
    on_submit: EventHandler<String>,
    on_input: EventHandler<String>,
) -> Element {
    let disabled = is_loading || !session_active || input_text.trim().is_empty();

    rsx! {
        div { class: "text-input-row",
            input {
                r#type: "text",
                placeholder: "Type a message...",
                value: "{input_text}",
                disabled: is_loading || !session_active,
                oninput: move |evt| on_input.call(evt.value()),
                onkeydown: {
                    let input_text = input_text.clone();
                    move |evt: Event<KeyboardData>| {
                        if evt.key() == Key::Enter && !evt.data().modifiers().shift() {
                            evt.prevent_default();
                            if !disabled {
                                on_submit.call(input_text.clone());
                            }
                        }
                    }
                },
            }
            button {
                class: "send-btn",
                disabled: disabled,
                onclick: move |_| {
                    if !disabled {
                        on_submit.call(input_text.clone());
                    }
                },
                svg { width: "16", height: "16", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
                    line { x1: "22", y1: "2", x2: "11", y2: "13" }
                    polygon { points: "22 2 15 22 11 13 2 9 22 2" }
                }
            }
        }
    }
}

#[component]
fn ModeToggle(
    current_mode: String,
    mic_available: bool,
    ffmpeg_available: bool,
    on_switch: EventHandler<String>,
) -> Element {
    let voice_disabled = !mic_available || !ffmpeg_available;
    rsx! {
        div { class: "mode-toggle",
            button {
                class: if current_mode == "text" { "mode-btn active" } else { "mode-btn" },
                onclick: move |_| on_switch.call("text".to_string()),
                "Text"
            }
            button {
                class: if current_mode == "voice" { "mode-btn active" } else { "mode-btn" },
                disabled: voice_disabled,
                onclick: move |_| on_switch.call("voice".to_string()),
                "Voice"
            }
        }
    }
}

#[component]
fn VoiceMode(
    is_recording: bool,
    is_loading: bool,
    session_active: bool,
    on_toggle: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "voice-mode",
            button {
                class: if is_recording { "mic-btn recording" } else { "mic-btn" },
                disabled: is_loading || !session_active,
                onclick: move |_| on_toggle.call(()),
                if is_recording {
                    svg { width: "24", height: "24", view_box: "0 0 24 24", fill: "white",
                        rect { x: "6", y: "6", width: "12", height: "12", rx: "2" }
                    }
                } else {
                    svg { width: "24", height: "24", view_box: "0 0 24 24", fill: "none", stroke: "white", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
                        path { d: "M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" }
                        path { d: "M19 10v2a7 7 0 0 1-14 0v-2" }
                        line { x1: "12", y1: "19", x2: "12", y2: "23" }
                        line { x1: "8", y1: "23", x2: "16", y2: "23" }
                    }
                }
            }
            span { class: "voice-hint",
                if is_recording { "Recording... tap to stop" } else { "Tap to speak" }
            }
        }
    }
}

#[component]
fn WarningBanner(mic_available: bool, ffmpeg_available: bool) -> Element {
    let text = if !mic_available {
        "Microphone not available. Please allow microphone access or use text mode."
    } else if !ffmpeg_available {
        "ffmpeg not found. Voice mode requires ffmpeg. Install with: brew install ffmpeg"
    } else {
        "Voice mode unavailable."
    };
    rsx! {
        div { class: "warning-banner", "{text}" }
    }
}

#[component]
fn ConnectingScreen(is_loading: bool) -> Element {
    rsx! {
        div { class: "connecting",
            div { class: "connecting-icon", "..." }
            div { "Connecting to Luna..." }
            if is_loading {
                div { class: "loading", style: "margin-top: 12px;",
                    div { class: "loading-dot" }
                    div { class: "loading-dot" }
                    div { class: "loading-dot" }
                }
            }
        }
    }
}

#[component]
pub fn App() -> Element {
    let mut messages = use_signal(Vec::<Message>::new);
    let mut input_text = use_signal(String::new);
    let mut is_loading = use_signal(|| false);
    let mut session_active = use_signal(|| false);
    let mut current_mode = use_signal(|| "text".to_string());
    let mut mic_available = use_signal(|| false);
    let mut ffmpeg_available = use_signal(|| true);
    let mut show_warning = use_signal(|| false);
    let mut is_recording = use_signal(|| false);
    let mut recording_handle: Signal<Option<JsValue>> = use_signal(|| None);

    use_effect(move || {
        spawn(async move {
            let window = web_sys::window().expect("no window");
            let navigator = window.navigator();
            if let Ok(media_devices) = navigator.media_devices() {
                let constraints = web_sys::MediaStreamConstraints::new();
                constraints.set_audio(&JsValue::from_bool(true));
                constraints.set_video(&JsValue::from_bool(false));
                if let Ok(promise) = media_devices.get_user_media_with_constraints(&constraints) {
                    if let Ok(result) = wasm_bindgen_futures::JsFuture::from(promise).await {
                        let stream: web_sys::MediaStream = result.into();
                        for i in 0..stream.get_tracks().length() {
                            if let Ok(track) = stream.get_tracks().get(i).dyn_into::<web_sys::MediaStreamTrack>() {
                                track.stop();
                            }
                        }
                        mic_available.set(true);
                        show_warning.set(false);
                        return;
                    }
                }
            }
            mic_available.set(false);
            show_warning.set(true);
            current_mode.set("text".to_string());
        });
    });

    use_effect(move || {
        spawn(async move {
            is_loading.set(true);
            match invoke_tauri("start_session", JsValue::NULL).await {
                Ok(value) => {
                    let ffmpeg = value.get("ffmpeg_available").and_then(|v| v.as_bool()).unwrap_or(false);
                    ffmpeg_available.set(ffmpeg);
                    if !ffmpeg || !mic_available() { show_warning.set(true); }
                    session_active.set(true);

                    // Fetch greeting in background (LLM call, takes a few seconds)
                    spawn(async move {
                        match invoke_tauri("get_greeting", JsValue::NULL).await {
                            Ok(greeting_val) => {
                                if let Some(greeting) = greeting_val.get("greeting").and_then(|g| g.as_str()) {
                                    messages.write().push(Message { role: MsgRole::Assistant, text: greeting.to_string() });
                                    trim_messages(&mut messages.write());
                                }
                            }
                            Err(e) => {
                                messages.write().push(Message { role: MsgRole::System, text: format!("Greeting failed: {}", e) });
                            }
                        }
                        is_loading.set(false);
                    });
                }
                Err(e) => {
                    messages.write().push(Message { role: MsgRole::System, text: format!("Failed to start session: {}", e) });
                    is_loading.set(false);
                }
            }
        });
    });

    let send_message = move |text: String| {
        if text.trim().is_empty() || is_loading() || !session_active() { return; }
        let text = text.trim().to_string();
        input_text.set(String::new());
        is_loading.set(true);
        messages.write().push(Message { role: MsgRole::User, text: text.clone() });
        trim_messages(&mut messages.write());

        spawn(async move {
            let args = serde_wasm_bindgen::to_value(&serde_json::json!({ "text": text })).unwrap();
            match invoke_tauri("send_message", args).await {
                Ok(result) => {
                    if let Some(reply) = result.get("reply").and_then(|r| r.as_str()) {
                        messages.write().push(Message { role: MsgRole::Assistant, text: reply.to_string() });
                        trim_messages(&mut messages.write());
                        if let Some(audio_file) = result.get("audio_file").and_then(|f| f.as_str()) {
                            if !audio_file.is_empty() {
                                let aargs = serde_wasm_bindgen::to_value(&serde_json::json!({ "path": audio_file })).unwrap();
                                if let Ok(b64) = invoke_tauri("read_audio_file", aargs).await {
                                    if let Some(s) = b64.as_str() { play_audio(s).await; }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    messages.write().push(Message { role: MsgRole::System, text: format!("Error: {}", e) });
                }
            }
            is_loading.set(false);
        });
    };

    let switch_mode = move |mode: String| {
        if mode == "voice" && (!mic_available() || !ffmpeg_available()) {
            messages.write().push(Message { role: MsgRole::System, text: "Voice mode not available.".to_string() });
            return;
        }
        current_mode.set(mode);
    };

    let toggle_recording = move |_| {
        if is_recording() {
            is_recording.set(false);
            is_loading.set(true);
            if let Some(h) = recording_handle() {
                spawn(async move {
                    let promise = stopRecording(&h);
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(data) => {
                            let base64 = js_sys::Reflect::get(&data, &JsValue::from_str("base64")).ok().and_then(|v| v.as_string()).unwrap_or_default();
                            let mime_type = js_sys::Reflect::get(&data, &JsValue::from_str("mimeType")).ok().and_then(|v| v.as_string()).unwrap_or_else(|| "audio/webm".to_string());
                            let args = serde_wasm_bindgen::to_value(&AudioArgs { audio_b64: base64, mime_type }).unwrap();
                            match invoke_tauri("transcribe_and_reply", args).await {
                                Ok(value) => {
                                    let transcription = value.get("transcription").and_then(|t| t.as_str()).unwrap_or("(unintelligible)");
                                    messages.write().push(Message { role: MsgRole::User, text: transcription.to_string() });
                                    trim_messages(&mut messages.write());
                                    if let Some(reply) = value.get("reply").and_then(|r| r.as_str()) {
                                        messages.write().push(Message { role: MsgRole::Assistant, text: reply.to_string() });
                                        trim_messages(&mut messages.write());
                                        if let Some(audio_file) = value.get("audio_file").and_then(|f| f.as_str()) {
                                            if !audio_file.is_empty() {
                                                let aargs = serde_wasm_bindgen::to_value(&serde_json::json!({ "path": audio_file })).unwrap();
                                                if let Ok(b64) = invoke_tauri("read_audio_file", aargs).await {
                                                    if let Some(s) = b64.as_str() { play_audio(s).await; }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    messages.write().push(Message { role: MsgRole::System, text: format!("Voice error: {}", e) });
                                }
                            }
                        }
                        Err(_e) => {
                            messages.write().push(Message { role: MsgRole::System, text: "Failed to process recording.".to_string() });
                        }
                    }
                    is_loading.set(false);
                    recording_handle.set(None);
                });
            }
        } else {
            spawn(async move {
                let promise = startRecording();
                match wasm_bindgen_futures::JsFuture::from(promise).await {
                    Ok(handle) => {
                        recording_handle.set(Some(handle));
                        is_recording.set(true);
                    }
                    Err(_e) => {
                        messages.write().push(Message { role: MsgRole::System, text: "Failed to start recording.".to_string() });
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "app",
            header { class: "header",
                div { class: "header-left",
                    div { class: "avatar", "L" }
                    div {
                        div { class: "header-title", "Chatterbox" }
                        div { class: "header-subtitle", "Chat with Luna" }
                    }
                }
                ModeToggle {
                    current_mode: current_mode(),
                    mic_available: mic_available(),
                    ffmpeg_available: ffmpeg_available(),
                    on_switch: switch_mode,
                }
            }

            if show_warning() {
                WarningBanner { mic_available: mic_available(), ffmpeg_available: ffmpeg_available() }
            }

            if !session_active() && messages.read().is_empty() {
                ConnectingScreen { is_loading: is_loading() }
            } else {
                MessageList { messages: messages() }
                if is_loading() {
                    LoadingIndicator {}
                }
            }

            if current_mode() == "text" {
                div { class: "input-area",
                    TextInput {
                        input_text: input_text(),
                        is_loading: is_loading(),
                        session_active: session_active(),
                        on_submit: send_message,
                        on_input: move |val| input_text.set(val),
                    }
                }
            } else {
                div { class: "input-area",
                    VoiceMode {
                        is_recording: is_recording(),
                        is_loading: is_loading(),
                        session_active: session_active(),
                        on_toggle: toggle_recording,
                    }
                }
            }
        }
    }
}

async fn play_audio(base64_wav: &str) {
    let data_url = format!("data:audio/wav;base64,{}", base64_wav);
    let audio = web_sys::HtmlAudioElement::new().unwrap();
    audio.set_src(&data_url);
    let _ = audio.play();
}
