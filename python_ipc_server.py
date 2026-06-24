#!/usr/bin/env python3
"""Python IPC Server for Chatterbox Desktop Client.

Receives commands via stdin (JSON-RPC style), processes them,
and returns results via stdout.

Protocol:
    Input:  {"method": "...", "params": {...}, "id": N}
    Output: {"id": N, "result": {...}}  or  {"id": N, "error": {...}}
"""

import sys
import os
import json
import subprocess
import tempfile
import base64
from typing import Optional, Dict, Any

# Add chatterbox project root to path so we can import chatterbox modules
# In dev: python_ipc_server.py is at chatterbox-desktop/, chatterbox package is at sibling chatterbox/chatterbox/
# In release: chatterbox/ is bundled alongside this script in Resources/
_script_dir = os.path.dirname(os.path.abspath(__file__))
_chatterbox_project = os.path.join(os.path.dirname(_script_dir), "chatterbox")
if not os.path.isdir(_chatterbox_project):
    # Release bundle: chatterbox is in the same directory as this script
    _chatterbox_project = _script_dir
sys.path.insert(0, _chatterbox_project)

# Deferred imports to avoid loading heavy modules at startup
_config = None
_llm = None
_tts = None


def _get_config():
    global _config
    if _config is None:
        from chatterbox.config import load_config
        _config = load_config()
    return _config


def _get_llm():
    global _llm
    if _llm is None:
        from chatterbox.llm.openai_adapter import OpenAILLM
        from chatterbox.llm.deepseek_adapter import DeepSeekLLM
        cfg = _get_config()
        engine = cfg["llm"]["engine"]
        if engine == "openai":
            _llm = OpenAILLM(api_key=os.environ.get("OPENAI_API_KEY", ""), model=cfg["llm"]["openai"]["model"])
        elif engine == "deepseek":
            _llm = DeepSeekLLM(api_key=os.environ.get("DEEPSEEK_API_KEY", ""), model=cfg["llm"]["deepseek"]["model"])
        else:
            raise ValueError(f"Unknown LLM engine: {engine}")
    return _llm


def _get_tts():
    global _tts
    if _tts is None:
        from chatterbox.tts.edge_tts import EdgeTTSEngine
        cfg = _get_config()
        _tts = EdgeTTSEngine(voice=cfg["tts"]["edge_tts"]["voice"])
    return _tts


def _synthesize_audio_to_file(text: str) -> Optional[str]:
    """Synthesize speech and write to a temp file, return the file path or None."""
    try:
        tts = _get_tts()
        audio = tts.synthesize(text)
        if audio:
            tmp = tempfile.NamedTemporaryFile(suffix=".wav", delete=False, prefix="tts_")
            tmp.write(audio)
            tmp_path = tmp.name
            tmp.close()
            sys.stderr.write(f"[Python] TTS audio written to {tmp_path}\n")
            sys.stderr.flush()
            return tmp_path
    except Exception as e:
        sys.stderr.write(f"[Python] TTS failed: {e}\n")
        sys.stderr.flush()
    return None


def check_ffmpeg() -> bool:
    """Check if ffmpeg is available on the system."""
    try:
        result = subprocess.run(
            ["ffmpeg", "-version"],
            capture_output=True,
            timeout=5,
        )
        return result.returncode == 0
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False


def convert_to_wav(audio_bytes: bytes, input_ext: str) -> bytes:
    """Convert audio bytes to WAV format using ffmpeg.

    Args:
        audio_bytes: Raw audio data in any format ffmpeg supports.
        input_ext: File extension hint (e.g. 'webm', 'ogg', 'mp4').

    Returns:
        WAV-encoded audio bytes.

    Raises:
        RuntimeError: If ffmpeg conversion fails.
    """
    with tempfile.NamedTemporaryFile(suffix=f".{input_ext}", delete=False) as tmp_in:
        tmp_in.write(audio_bytes)
        tmp_in_path = tmp_in.name

    tmp_out_path = tmp_in_path + ".wav"
    try:
        result = subprocess.run(
            [
                "ffmpeg", "-y", "-i", tmp_in_path,
                "-acodec", "pcm_s16le",
                "-ar", "16000",
                "-ac", "1",
                tmp_out_path,
            ],
            capture_output=True,
            timeout=30,
        )
        if result.returncode != 0:
            raise RuntimeError(f"ffmpeg failed: {result.stderr.decode('utf-8', errors='replace')[:500]}")

        with open(tmp_out_path, "rb") as f:
            return f.read()
    finally:
        for path in (tmp_in_path, tmp_out_path):
            try:
                os.unlink(path)
            except OSError:
                pass


class Session:
    """Holds conversation state for one session."""

    def __init__(self, profile_data: Optional[Dict[str, Any]]):
        from chatterbox.profile import UserProfile, ProfileStore
        from chatterbox.conversation.manager import ConversationManager

        sys.stderr.write("[Python] Session init: loading profile...\n")
        sys.stderr.flush()
        if profile_data:
            self.profile = UserProfile.from_dict(profile_data)
        else:
            self.profile = ProfileStore().get_default() or UserProfile()

        strategy = _get_config().get("conversation", {}).get("strategy", "beginner")
        persona_name = _get_config().get("persona", {}).get("name", "Luna")

        self.conversation = ConversationManager(
            strategy=strategy,
            persona_name=persona_name,
            user_profile=self.profile,
            english_level=self.profile.english_level,
        )

        # Lazy-load the LLM and do greeting
        sys.stderr.write("[Python] Session init: loading LLM...\n")
        sys.stderr.flush()
        llm = _get_llm()
        greeting_prompt = self.conversation.get_greeting_prompt()
        self.conversation.add_user_message(greeting_prompt)
        sys.stderr.write("[Python] Session init: getting greeting from LLM...\n")
        sys.stderr.flush()
        greeting_reply = llm.chat(self.conversation.get_messages())
        sys.stderr.write("[Python] Session init: greeting received\n")
        sys.stderr.flush()
        self.conversation.add_assistant_message(greeting_reply)

        self.profile_store = ProfileStore()


_session: Optional["Session"] = None


def handle_start_session(params: dict) -> dict:
    """Initialize session without waiting for greeting (fast)."""
    global _session
    profile_data = params.get("profile")
    _session = Session.__new__(Session)
    from chatterbox.profile import UserProfile, ProfileStore
    from chatterbox.conversation.manager import ConversationManager

    sys.stderr.write("[Python] Session init: loading profile...\n")
    sys.stderr.flush()
    if profile_data:
        _session.profile = UserProfile.from_dict(profile_data)
    else:
        _session.profile = ProfileStore().get_default() or UserProfile()

    cfg = _get_config()
    strategy = cfg.get("conversation", {}).get("strategy", "beginner")
    persona_name = cfg.get("persona", {}).get("name", "Luna")

    _session.conversation = ConversationManager(
        strategy=strategy,
        persona_name=persona_name,
        user_profile=_session.profile,
        english_level=_session.profile.english_level,
    )
    _session.profile_store = ProfileStore()

    # Pre-load LLM in background (don't block)
    sys.stderr.write("[Python] Session init: pre-loading LLM...\n")
    sys.stderr.flush()
    _get_llm()  # This caches the LLM instance

    return {
        "status": "ok",
        "profile": _session.profile.to_dict() if _session.profile else None,
        "greeting": None,  # Will be fetched separately
        "ffmpeg_available": check_ffmpeg(),
    }


def handle_get_greeting(params: dict) -> dict:
    """Generate greeting from LLM (slow, called after session init)."""
    if _session is None:
        raise RuntimeError("No active session.")

    sys.stderr.write("[Python] Getting greeting from LLM...\n")
    sys.stderr.flush()
    llm = _get_llm()
    greeting_prompt = _session.conversation.get_greeting_prompt()
    _session.conversation.add_user_message(greeting_prompt)
    greeting_reply = llm.chat(_session.conversation.get_messages())
    _session.conversation.add_assistant_message(greeting_reply)
    sys.stderr.write("[Python] Greeting received\n")
    sys.stderr.flush()

    return {"greeting": greeting_reply}


def handle_add_message(params: dict) -> dict:
    if _session is None:
        raise RuntimeError("No active session. Call start_session first.")

    text = params.get("text", "")
    if not text:
        return {"reply": "", "translation": "", "audio": None}

    # Add user message (triggers repetition detection)
    _session.conversation.add_user_message(text)

    # Get LLM reply
    llm = _get_llm()
    try:
        reply = llm.chat(_session.conversation.get_messages())
    except Exception as e:
        return {"reply": f"Sorry, I encountered an error: {e}", "translation": "", "audio": None}

    _session.conversation.add_assistant_message(reply)

    # Parse translation from reply if present
    from chatterbox.conversation.manager import ConversationManager
    translation, response = ConversationManager.parse_translation_reply(reply)

    # Synthesize audio to file
    audio_file = _synthesize_audio_to_file(response or reply)

    return {
        "reply": reply,
        "translation": translation,
        "response": response,
        "audio_file": audio_file,
    }


def handle_quit(params: dict) -> dict:
    global _session
    if _session is None:
        return {"status": "ok"}

    # Save profile
    try:
        _session.profile_store.save_default(_session.profile)
    except Exception:
        pass

    _session = None
    return {"status": "ok"}


def handle_switch_mode(params: dict) -> dict:
    """Switch between voice and text mode (placeholder for now)."""
    mode = params.get("mode", "text")
    return {"mode": mode, "status": "ok"}


def handle_transcribe_and_reply(params: dict) -> dict:
    """Receive audio file path → ffmpeg → ASR → LLM → TTS → return full result."""
    if _session is None:
        raise RuntimeError("No active session. Call start_session first.")

    audio_file = params.get("audio_file", "")
    mime_type = params.get("mime_type", "audio/webm")

    if not audio_file or not os.path.isfile(audio_file):
        return {"reply": "", "transcription": "", "audio_file": None}

    # 1. Read audio from file
    with open(audio_file, "rb") as f:
        audio_bytes = f.read()
    # Clean up input temp file
    try:
        os.unlink(audio_file)
    except OSError:
        pass

    sys.stderr.write(f"[Python] Received audio: {len(audio_bytes)} bytes, mime={mime_type}\n")
    sys.stderr.flush()

    # 2. Determine input extension from mime type
    ext_map = {
        "audio/webm": "webm",
        "audio/ogg": "ogg",
        "audio/mp4": "mp4",
        "audio/mpeg": "mp3",
        "audio/wav": "wav",
        "audio/x-wav": "wav",
    }
    input_ext = ext_map.get(mime_type, "webm")

    # 3. Convert to WAV if needed
    if input_ext != "wav":
        sys.stderr.write(f"[Python] Converting {input_ext} to WAV...\n")
        sys.stderr.flush()
        try:
            wav_bytes = convert_to_wav(audio_bytes, input_ext)
        except Exception as e:
            sys.stderr.write(f"[Python] ffmpeg conversion failed: {e}\n")
            sys.stderr.flush()
            return {"reply": "Sorry, I couldn't process the audio. Please try text mode.", "transcription": "", "audio_file": None}
    else:
        wav_bytes = audio_bytes

    # 4. ASR transcription
    sys.stderr.write("[Python] Transcribing audio...\n")
    sys.stderr.flush()
    cfg = _get_config()
    asr_engine = cfg.get("asr", {}).get("engine", "whisper-api")

    if asr_engine == "whisper-api":
        from chatterbox.asr.whisper import WhisperAPIASR
        asr = WhisperAPIASR(
            api_key=os.environ.get("OPENAI_API_KEY", ""),
            model=cfg.get("asr", {}).get("whisper_api", {}).get("model", "whisper-1"),
        )
    elif asr_engine == "whisper-local":
        from chatterbox.asr.whisper_local import WhisperLocalASR
        asr = WhisperLocalASR(
            model_size=cfg.get("asr", {}).get("whisper_local", {}).get("model_size", "base"),
            device=cfg.get("asr", {}).get("whisper_local", {}).get("device", "auto"),
        )
    else:
        raise ValueError(f"Unknown ASR engine: {asr_engine}")

    try:
        transcription = asr.transcribe(wav_bytes)
    except Exception as e:
        sys.stderr.write(f"[Python] ASR failed: {e}\n")
        sys.stderr.flush()
        return {"reply": "Sorry, I couldn't understand the audio. Please try again.", "transcription": "", "audio_file": None}

    sys.stderr.write(f"[Python] Transcription: {transcription}\n")
    sys.stderr.flush()

    if not transcription.strip():
        return {"reply": "I didn't hear anything. Please try again.", "transcription": "", "audio_file": None}

    # 5. Add user message and get LLM reply (same as handle_add_message)
    _session.conversation.add_user_message(transcription)
    llm = _get_llm()
    try:
        reply = llm.chat(_session.conversation.get_messages())
    except Exception as e:
        return {"reply": f"Sorry, I encountered an error: {e}", "transcription": transcription, "audio_file": None}

    _session.conversation.add_assistant_message(reply)

    from chatterbox.conversation.manager import ConversationManager
    translation, response = ConversationManager.parse_translation_reply(reply)

    audio_file_out = _synthesize_audio_to_file(response or reply)

    return {
        "reply": reply,
        "transcription": transcription,
        "translation": translation,
        "response": response,
        "audio_file": audio_file_out,
    }


HANDLERS = {
    "start_session": handle_start_session,
    "get_greeting": handle_get_greeting,
    "add_message": handle_add_message,
    "quit": handle_quit,
    "switch_mode": handle_switch_mode,
    "transcribe_and_reply": handle_transcribe_and_reply,
}


def write_response(resp: dict):
    """Write a JSON response to stdout."""
    line = json.dumps(resp, ensure_ascii=False)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def main():
    """Read commands from stdin and dispatch to handlers."""
    sys.stderr.write("[Python] IPC server started\n")
    sys.stderr.flush()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        sys.stderr.write(f"[Python] Received: {line}\n")
        sys.stderr.flush()
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            write_response({"id": None, "error": {"code": -32700, "message": "Invalid JSON"}})
            continue

        method = msg.get("method")
        params = msg.get("params", {})
        msg_id = msg.get("id")

        handler = HANDLERS.get(method)
        if handler is None:
            write_response({"id": msg_id, "error": {"code": -32601, "message": f"Unknown method: {method}"}})
            continue

        try:
            sys.stderr.write(f"[Python] Handling {method}...\n")
            sys.stderr.flush()
            result = handler(params)
            sys.stderr.write(f"[Python] {method} done\n")
            sys.stderr.flush()
            write_response({"id": msg_id, "result": result})
        except Exception as e:
            sys.stderr.write(f"[Python] {method} error: {e}\n")
            sys.stderr.flush()
            write_response({"id": msg_id, "error": {"code": -32603, "message": str(e)}})


if __name__ == "__main__":
    main()
