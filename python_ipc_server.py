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
from typing import Optional, Dict, Any

# Add chatterbox project root to path so we can import chatterbox modules
# python_ipc_server.py is at chatterbox-desktop/, chatterbox package is at sibling chatterbox/chatterbox/
_project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_chatterbox_project = os.path.join(_project_root, "chatterbox")
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


def _synthesize_audio(text: str) -> Optional[str]:
    """Synthesize speech and return base64-encoded WAV data, or None on failure."""
    try:
        tts = _get_tts()
        audio = tts.synthesize(text)
        if audio:
            import base64
            return base64.b64encode(audio).decode("utf-8")
    except Exception:
        pass
    return None


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
    global _session
    profile_data = params.get("profile")
    _session = Session(profile_data)
    return {
        "status": "ok",
        "profile": _session.profile.to_dict() if _session.profile else None,
        "greeting": _session.conversation.messages[-1]["content"] if _session.conversation.messages else None,
    }


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

    # Synthesize audio
    audio_b64 = _synthesize_audio(response or reply)

    return {
        "reply": reply,
        "translation": translation,
        "response": response,
        "audio": audio_b64,
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


HANDLERS = {
    "start_session": handle_start_session,
    "add_message": handle_add_message,
    "quit": handle_quit,
    "switch_mode": handle_switch_mode,
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
