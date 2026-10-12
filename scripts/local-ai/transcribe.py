"""Offline MLX Whisper adapter. Models must be installed explicitly before use."""
import contextlib
import json
import os
from pathlib import Path
import sys

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"


def emit(event):
    print(json.dumps(event, ensure_ascii=False), flush=True)


def main():
    request = json.loads(sys.stdin.buffer.read(1_000_001))
    model = Path(request.get("model", ""))
    if not model.is_absolute() or not model.is_dir() or not (model / "config.json").is_file():
        emit({"error": "mlx_model_missing"})
        return
    # Third-party progress output must not corrupt the protocol or expose inputs.
    with contextlib.redirect_stdout(sys.stderr):
        import mlx_whisper
        result = mlx_whisper.transcribe(
            request["audio_path"], path_or_hf_repo=str(model),
            language=request.get("locale", "en-US").split("-")[0],
            verbose=None,
        )
    emit({"delta": result["text"]})
    emit({"done": True})


if __name__ == "__main__":
    try:
        main()
    except Exception:
        emit({"error": "worker_failed"})
