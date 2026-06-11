use leptos::task::spawn_local;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

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

#[derive(Serialize, Deserialize)]
struct TextArgs {
    text: String,
}

#[derive(Clone, Debug)]
enum MsgRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug)]
struct Message {
    role: MsgRole,
    text: String,
}

#[component]
pub fn App() -> impl IntoView {
    // ─── State ───────────────────────────────────────────────────────────────
    let (messages, set_messages) = signal(Vec::<Message>::new());
    let (input_text, set_input_text) = signal(String::new());
    let (is_loading, set_is_loading) = signal(false);
    let (session_active, set_session_active) = signal(false);
    let (current_mode, set_current_mode) = signal("text".to_string());
    let (mic_available, set_mic_available) = signal(false);
    let (show_warning, set_show_warning) = signal(false);
    let (is_recording, set_is_recording) = signal(false);

    // ─── Effects ─────────────────────────────────────────────────────────────

    // Check devices on mount (simplified - just set to false for now)
    Effect::new(move |_| {
        set_mic_available.set(false);
        set_show_warning.set(true);
        set_current_mode.set("text".to_string());
    });

    // Start session on mount
    Effect::new(move |_| {
        spawn_local(async move {
            start_chat_session(&set_messages, &set_session_active, &set_is_loading).await;
        });
    });

    // ─── Helpers ─────────────────────────────────────────────────────────────

    async fn start_chat_session(
        set_messages: &WriteSignal<Vec<Message>>,
        set_session_active: &WriteSignal<bool>,
        set_is_loading: &WriteSignal<bool>,
    ) {
        set_is_loading.set(true);
        web_sys::console::log_1(&"[FE] Starting session...".into());
        let args = JsValue::NULL;
        let result = invokeSafe("start_session", args).await;
        web_sys::console::log_1(&format!("[FE] invokeSafe result: {:?}", result).into());
        match serde_wasm_bindgen::from_value::<serde_json::Value>(result) {
            Ok(wrapped) => {
                web_sys::console::log_1(&format!("[FE] parsed wrapped: {:?}", wrapped).into());
                if wrapped.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let result = wrapped.get("value").cloned().unwrap_or(serde_json::Value::Null);
                    if let Some(greeting) = result.get("greeting").and_then(|g| g.as_str()) {
                        set_messages.update(|msgs| {
                            msgs.push(Message {
                                role: MsgRole::Assistant,
                                text: greeting.to_string(),
                            });
                        });
                    }
                    set_session_active.set(true);
                } else {
                    let error = wrapped.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
                    web_sys::console::log_1(&format!("[FE] start_session error: {}", error).into());
                    set_messages.update(|msgs| {
                        msgs.push(Message {
                            role: MsgRole::System,
                            text: format!("Failed to start session: {}", error),
                        });
                    });
                }
            }
            Err(e) => {
                web_sys::console::log_1(&format!("[FE] parse error: {:?}", e).into());
                set_messages.update(|msgs| {
                    msgs.push(Message {
                        role: MsgRole::System,
                        text: format!("Failed to connect to Luna: {:?}. Please restart the app.", e),
                    });
                });
            }
        }
        set_is_loading.set(false);
    }

    let send_message = move |_: leptos::ev::MouseEvent| {
        let text = input_text.get_untracked();
        if text.trim().is_empty() || is_loading.get_untracked() || !session_active.get_untracked() {
            return;
        }

        let text = text.trim().to_string();
        set_input_text.set(String::new());
        set_is_loading.set(true);

        set_messages.update(|msgs| {
            msgs.push(Message {
                role: MsgRole::User,
                text: text.clone(),
            });
        });

        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&TextArgs { text }).unwrap();
            let result = invokeSafe("send_message", args).await;
            match serde_wasm_bindgen::from_value::<serde_json::Value>(result) {
                Ok(wrapped) => {
                    if wrapped.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        let result = wrapped.get("value").cloned().unwrap_or(serde_json::Value::Null);
                        if let Some(reply) = result.get("reply").and_then(|r| r.as_str()) {
                            set_messages.update(|msgs| {
                                msgs.push(Message {
                                    role: MsgRole::Assistant,
                                    text: reply.to_string(),
                                });
                            });

                            // Play audio if available
                            if let Some(audio_b64) = result.get("audio").and_then(|a| a.as_str()) {
                                if !audio_b64.is_empty() {
                                    play_audio(audio_b64).await;
                                }
                            }
                        }
                    } else {
                        let error = wrapped.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
                        set_messages.update(|msgs| {
                            msgs.push(Message {
                                role: MsgRole::System,
                                text: format!("Error: {}", error),
                            });
                        });
                    }
                }
                Err(e) => {
                    set_messages.update(|msgs| {
                        msgs.push(Message {
                            role: MsgRole::System,
                            text: format!("Failed to get response: {:?}. Please try again.", e),
                        });
                    });
                }
            }
            set_is_loading.set(false);
        });
    };

    let handle_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            // Create a dummy mouse event for the handler
            let dummy_event = leptos::ev::MouseEvent::new("click").unwrap();
            send_message(dummy_event);
        }
    };

    let switch_mode = move |mode: String| {
        if mode == "voice" && !mic_available.get_untracked() {
            set_messages.update(|msgs| {
                msgs.push(Message {
                    role: MsgRole::System,
                    text: "Microphone not available. Please use text mode.".to_string(),
                });
            });
            return;
        }
        set_current_mode.set(mode);
    };

    let toggle_recording = move |_: leptos::ev::MouseEvent| {
        if is_recording.get_untracked() {
            set_is_recording.set(false);
            set_messages.update(|msgs| {
                msgs.push(Message {
                    role: MsgRole::System,
                    text: "(Voice input not yet integrated. Please use text mode.)".to_string(),
                });
            });
        } else {
            set_is_recording.set(true);
        }
    };

    // ─── View ────────────────────────────────────────────────────────────────

    view! {
        <div class="app">
            // Header
            <header class="header">
                <div class="header-left">
                    <div class="avatar">"🌙"</div>
                    <div>
                        <div class="header-title">"Chatterbox"</div>
                        <div class="header-subtitle">"Chat with Luna"</div>
                    </div>
                </div>
                <div class="mode-toggle">
                    <button
                        class={move || if current_mode.get() == "text" { "mode-btn active" } else { "mode-btn" }}
                        on:click={move |_| switch_mode("text".to_string())}
                    >
                        "⌨️ Text"
                    </button>
                    <button
                        class={move || if current_mode.get() == "voice" { "mode-btn active" } else { "mode-btn" }}
                        on:click={move |_| switch_mode("voice".to_string())}
                        disabled={move || !mic_available.get()}
                    >
                        "🎤 Voice"
                    </button>
                </div>
            </header>

            // Warning Banner
            {move || if show_warning.get() {
                view! {
                    <div class="warning-banner">
                        "⚠️ Voice mode unavailable. Please allow microphone access or use text mode."
                    </div>
                }.into_any()
            } else {
                view! { <div></div> }.into_any()
            }}

            // Messages
            <div class="messages">
                {move || if !session_active.get() {
                    view! {
                        <div class="connecting">
                            <div class="connecting-icon">"🤖"</div>
                            <div>"Connecting to Luna..."</div>
                            {move || if is_loading.get() {
                                view! {
                                    <div class="loading" style="margin-top: 12px;">
                                        <div class="loading-dot"></div>
                                        <div class="loading-dot"></div>
                                        <div class="loading-dot"></div>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <div></div> }.into_any()
                            }}
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div>
                            {move || messages.get().into_iter().map(|msg| {
                                let class_name = match msg.role {
                                    MsgRole::User => "message user",
                                    MsgRole::Assistant => "message assistant",
                                    MsgRole::System => "message system",
                                };
                                view! {
                                    <div class={class_name}>
                                        <div class="bubble">
                                            {match msg.role {
                                                MsgRole::Assistant => view! {
                                                    <>
                                                        <div class="name">"Luna"</div>
                                                        <div>{msg.text}</div>
                                                    </>
                                                }.into_any(),
                                                _ => view! { <div>{msg.text}</div> }.into_any(),
                                            }}
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}

                            {move || if is_loading.get() {
                                view! {
                                    <div class="message assistant">
                                        <div class="bubble">
                                            <div class="name">"Luna"</div>
                                            <div class="loading">
                                                <div class="loading-dot"></div>
                                                <div class="loading-dot"></div>
                                                <div class="loading-dot"></div>
                                            </div>
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                view! { <div></div> }.into_any()
                            }}
                        </div>
                    }.into_any()
                }}
            </div>

            // Input Area
            <div class="input-area">
                {move || if current_mode.get() == "text" {
                    view! {
                        <div class="text-input-row">
                            <input
                                type="text"
                                placeholder="Type a message..."
                                prop:value={move || input_text.get()}
                                on:input={move |ev| {
                                    let val = event_target_value(&ev);
                                    set_input_text.set(val);
                                }}
                                on:keydown={handle_keydown}
                                disabled={move || is_loading.get() || !session_active.get()}
                            />
                            <button
                                class="send-btn"
                                on:click={send_message}
                                disabled={move || input_text.get().trim().is_empty() || is_loading.get() || !session_active.get()}
                            >
                                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <line x1="22" y1="2" x2="11" y2="13"></line>
                                    <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
                                </svg>
                            </button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="voice-mode">
                            <button
                                class={move || if is_recording.get() { "mic-btn recording" } else { "mic-btn" }}
                                on:click={toggle_recording}
                                disabled={move || is_loading.get() || !session_active.get()}
                            >
                                {move || if is_recording.get() {
                                    view! {
                                        <svg width="24" height="24" viewBox="0 0 24 24" fill="white">
                                            <rect x="6" y="6" width="12" height="12" rx="2"/>
                                        </svg>
                                    }.into_any()
                                } else {
                                    view! {
                                        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                            <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z"></path>
                                            <path d="M19 10v2a7 7 0 0 1-14 0v-2"></path>
                                            <line x1="12" y1="19" x2="12" y2="23"></line>
                                            <line x1="8" y1="23" x2="16" y2="23"></line>
                                        </svg>
                                    }.into_any()
                                }}
                            </button>
                            <span class="voice-hint">
                                {move || if is_recording.get() { "Recording... tap to stop" } else { "Tap to speak" }}
                            </span>
                        </div>
                    }.into_any()
                }}
            </div>
        </div>
    }
}

async fn play_audio(base64_wav: &str) {
    // Use data URL directly
    let data_url = format!("data:audio/wav;base64,{}", base64_wav);
    let audio = web_sys::HtmlAudioElement::new().unwrap();
    audio.set_src(&data_url);
    let _ = audio.play();
}

fn event_target_value(ev: &leptos::ev::Event) -> String {
    ev.target()
        .unwrap()
        .dyn_into::<web_sys::HtmlInputElement>()
        .unwrap()
        .value()
}
