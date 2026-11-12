#!/usr/bin/env python3
"""验证 MJ FAST/RELAX Imagine / v8.1、Blend 和 Upscale；密钥通过环境变量或隐藏输入提供。"""

from __future__ import annotations

import argparse
import base64
import getpass
import json
import os
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


BASE_URL = "https://magic666.top"
MODEL = "mj_fast_imagine"
RELAX_MODEL = "mj_relax_imagine"
MJ_ROUTE_PREFIXES = {"FAST": "/mj", "RELAX": "/mj-relax/mj"}
BLEND_MODEL = "mj_fast_blend"
UPSCALE_MODEL = "mj_fast_upscale"
UPSAMPLE_ACTION_MODEL = "mj_fast_upsample_action"
POLL_SECONDS = 10
TERMINAL_FAILURES = {"FAILURE", "FAILED", "CANCEL", "CANCELLED"}
PROMPTS = {
    "text": "A small orange ceramic fox with a blue scarf, sitting on a wooden table, warm studio photography, plain cream background --ar 1:1 --v 8.1",
    "base64": "An orange ceramic fox with a blue scarf inspired by the reference image, sitting beside a tiny green plant, bright studio photography --iw 1.5 --ar 1:1 --v 8.1",
    "url": "An orange ceramic fox with a blue scarf inspired by the reference image, on a windowsill on a rainy day, studio photography --iw 1.5 --ar 1:1 --v 8.1",
    "style": "A ceramic rabbit with a green scarf on a wooden table, studio photography --ar 1:1 --v 8.1",
}


def build_body(
    case: str,
    reference: str | list[str] | None,
    prompt: str | None = None,
    mode: str = "FAST",
) -> dict:
    if mode not in {"FAST", "RELAX"}:
        raise ValueError("mode must be FAST or RELAX")
    references = [reference] if isinstance(reference, str) else reference or []
    if case == "blend":
        if not 2 <= len(references) <= 5:
            raise ValueError("blend requires 2 to 5 reference images")
        if prompt is not None:
            raise ValueError(
                "blend does not accept --prompt; use Imagine for text guidance"
            )
        body = {"mode": mode, "dimensions": "SQUARE"}
    else:
        body = {"mode": mode, "prompt": PROMPTS[case] if prompt is None else prompt}
        if mode == "RELAX":
            parameters = body["prompt"].split()
            if "--fast" in parameters or "--turbo" in parameters:
                raise ValueError("RELAX mode conflicts with --fast or --turbo")
            if "--relax" not in parameters:
                body["prompt"] += " --relax"
    if case != "text" and not references:
        raise ValueError(f"{case} requires --reference")
    if case in {"base64", "blend"}:
        body["base64Array"] = []
        for reference in references:
            raw = Path(reference).read_bytes()
            if raw.startswith(b"\x89PNG\r\n\x1a\n"):
                mime = "image/png"
            elif raw.startswith(b"\xff\xd8\xff"):
                mime = "image/jpeg"
            else:
                raise ValueError("reference must be a PNG or JPEG image")
            body["base64Array"].append(
                f"data:{mime};base64,{base64.b64encode(raw).decode()}"
            )
    elif case in {"url", "style"}:
        if any(
            not reference.startswith("https://") or any(c.isspace() for c in reference)
            for reference in references
        ):
            raise ValueError(
                "reference must be a public HTTPS image URL without whitespace"
            )
        urls = " ".join(references)
        body["prompt"] = (
            f"{urls} {body['prompt']}"
            if case == "url"
            else f"{body['prompt']} --sref {urls}"
        )
    return body


def submit_task_id(body: dict) -> str:
    task_id = body.get("result")
    if (
        body.get("code") not in {1, 21, 22}
        or not isinstance(task_id, str)
        or not task_id
    ):
        raise RuntimeError(
            f"submission did not return a task ID: {json.dumps(body, ensure_ascii=False)}"
        )
    return task_id


def build_upscale_body(parent_task_id: str | None, index: int) -> dict:
    if not parent_task_id or not parent_task_id.strip():
        raise ValueError("upscale requires --parent-task-id")
    if index not in range(1, 5):
        raise ValueError("upscale index must be 1, 2, 3, or 4")
    return {
        "mode": "FAST",
        "taskId": parent_task_id,
        "action": "UPSCALE",
        "index": index,
    }


def build_upscale_action_body(parent: dict, scale: int) -> dict:
    if scale not in {2, 4}:
        raise ValueError("upscale scale must be 2 or 4")
    if parent.get("status") != "SUCCESS" or not parent.get("id"):
        raise ValueError("upscale requires a successful source task")
    label = f"Upscale ({scale}x)"
    for button in parent.get("buttons") or []:
        if isinstance(button, dict) and button.get("label") == label:
            custom_id = button.get("customId")
            if isinstance(custom_id, str) and custom_id:
                return {"mode": "FAST", "taskId": parent["id"], "customId": custom_id}
    raise ValueError(f"source task has no {label} button; select U1..U4 first")


def request_json(path: str, key: str, body: dict | None = None) -> dict:
    request = urllib.request.Request(
        BASE_URL + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=90) as response:
            result = json.load(response)
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace").replace(key, "[REDACTED]")
        raise RuntimeError(f"HTTP {exc.code}: {detail}") from None
    if not isinstance(result, dict):
        raise RuntimeError("API response must be a JSON object")
    return result


def save_json(path: Path, body: dict, key: str) -> None:
    path.write_text(
        json.dumps(body, indent=2, ensure_ascii=False).replace(key, "[REDACTED]") + "\n"
    )


def self_test() -> None:
    assert build_body("text", None) == {"mode": "FAST", "prompt": PROMPTS["text"]}
    assert build_body("text", None, "中文提示词 --v 8.1", "RELAX") == {
        "mode": "RELAX",
        "prompt": "中文提示词 --v 8.1 --relax",
    }
    assert (
        build_body("text", None, "中文提示词 --relax", "RELAX")["prompt"]
        == "中文提示词 --relax"
    )
    try:
        build_body("text", None, "中文提示词 --fast", "RELAX")
    except ValueError:
        pass
    else:
        raise AssertionError("conflicting mode parameters accepted")
    try:
        build_body("text", None, mode="INVALID")
    except ValueError:
        pass
    else:
        raise AssertionError("invalid processing mode accepted")
    assert build_upscale_body("123", 4) == {
        "mode": "FAST",
        "taskId": "123",
        "action": "UPSCALE",
        "index": 4,
    }
    for parent, index in ((None, 1), ("", 1), ("123", 0), ("123", 5)):
        try:
            build_upscale_body(parent, index)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid upscale target accepted")
    source = {
        "id": "123",
        "status": "SUCCESS",
        "buttons": [{"label": "Upscale (2x)", "customId": "server-provided-id"}],
    }
    assert build_upscale_action_body(source, 2) == {
        "mode": "FAST",
        "taskId": "123",
        "customId": "server-provided-id",
    }
    for task, scale in ((source, 4), (source, 3), ({**source, "status": "FAILURE"}, 2)):
        try:
            build_upscale_action_body(task, scale)
        except ValueError:
            pass
        else:
            raise AssertionError("unavailable upscale action accepted")
    with tempfile.TemporaryDirectory(prefix="mj-smoke-self-test-") as directory:
        reference = Path(directory) / "reference.png"
        raw = b"\x89PNG\r\n\x1a\n"
        reference.write_bytes(raw)
        encoded = build_body("base64", str(reference))["base64Array"][0]
        assert encoded.startswith("data:image/png;base64,")
        assert base64.b64decode(encoded.split(",", 1)[1]) == raw
        blend = build_body("blend", [str(reference), str(reference)])
        assert len(blend["base64Array"]) == 2
        assert blend["mode"] == "FAST" and "prompt" not in blend
        assert blend["dimensions"] == "SQUARE"
        for count in (0, 1, 6):
            try:
                build_body("blend", [str(reference)] * count)
            except ValueError:
                pass
            else:
                raise AssertionError("invalid blend reference count accepted")
        reference.write_bytes(b"not an image")
        try:
            build_body("base64", str(reference))
        except ValueError:
            pass
        else:
            raise AssertionError("invalid image signature accepted")
    assert build_body("url", "https://example.com/ref.png")["prompt"].startswith(
        "https://"
    )
    assert (
        "--sref https://example.com/ref.png"
        in build_body("style", "https://example.com/ref.png")["prompt"]
    )
    urls = ["https://example.com/fox.png", "https://example.com/rabbit.png"]
    assert build_body("url", urls, "two animals --v 8.1")["prompt"] == (
        " ".join(urls) + " two animals --v 8.1"
    )
    assert build_body("style", urls)["prompt"].endswith("--sref " + " ".join(urls))
    for code in (1, 21, 22):
        assert submit_task_id({"code": code, "result": "123"}) == "123"
    for response in (
        {"code": 23, "result": "123"},
        {"code": 1, "result": []},
        {"code": 3, "result": None},
    ):
        try:
            submit_task_id(response)
        except RuntimeError:
            pass
        else:
            raise AssertionError("invalid submission accepted")
    for reference in (None, "http://example.com/ref.png", "https://example.com/a b"):
        try:
            build_body("url", reference)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid reference accepted")
    print("self-test passed", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case", choices=[*PROMPTS, "blend", "upscale", "upscale-hd"], default="text"
    )
    parser.add_argument(
        "--mode",
        choices=["FAST", "RELAX"],
        default="FAST",
        help="Imagine processing mode",
    )
    parser.add_argument(
        "--reference",
        nargs="+",
        help="base64/blend: PNG/JPEG paths; url/style: public HTTPS image URLs",
    )
    parser.add_argument(
        "--prompt", help="override the Imagine prompt; not supported for Blend"
    )
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument(
        "--parent-task-id", help="successful source task ID for Upscale"
    )
    parser.add_argument(
        "--index", type=int, choices=range(1, 5), default=1, help="U1..U4"
    )
    parser.add_argument(
        "--scale", type=int, choices=[2, 4], default=2, help="HD upscale factor"
    )
    parser.add_argument(
        "--task-id", help="resume polling without submitting another billed generation"
    )
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--models-only", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.mode == "RELAX" and args.case not in PROMPTS:
        parser.error("--mode RELAX is currently supported for Imagine cases only")
    imagine_model = MODEL if args.mode == "FAST" else RELAX_MODEL
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if not args.models_only and args.output_dir is None:
        parser.error("--output-dir is required")
    body = None
    if not args.models_only and not args.task_id:
        if args.case == "upscale-hd":
            if not args.parent_task_id:
                parser.error("--parent-task-id is required for upscale-hd")
        else:
            body = (
                build_upscale_body(args.parent_task_id, args.index)
                if args.case == "upscale"
                else build_body(args.case, args.reference, args.prompt, args.mode)
            )
    key = os.environ.get("MIDJOURNEY_API_KEY") or getpass.getpass(
        "Midjourney API key: "
    )
    if not key.strip():
        parser.error("API key is empty")
    if args.models_only:
        result = request_json("/v1/models", key)
        models = [
            item["id"]
            for item in result.get("data", [])
            if isinstance(item, dict) and "id" in item
        ]
        print(
            json.dumps(
                {
                    "requested_model": imagine_model,
                    "available": imagine_model in models,
                    "mj_models": [m for m in models if m.startswith("mj_")],
                },
                ensure_ascii=False,
            ),
            flush=True,
        )
        return
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    if args.task_id:
        task_id = args.task_id
    else:
        if args.case in {"upscale", "upscale-hd"}:
            parent = request_json(f"/mj/task/{args.parent_task_id}/fetch", key)
            save_json(output / "parent.json", parent, key)
            if parent.get("status") != "SUCCESS":
                raise RuntimeError(
                    f"source task is not successful: {parent.get('status')}"
                )
            if args.case == "upscale-hd":
                body = build_upscale_action_body(parent, args.scale)
        save_json(output / "request.json", body, key)
        action = {"blend": "blend", "upscale": "change", "upscale-hd": "action"}.get(
            args.case, "imagine"
        )
        submitted = request_json(
            f"{MJ_ROUTE_PREFIXES[args.mode]}/submit/{action}", key, body
        )
        save_json(output / "submit.json", submitted, key)
        print(
            json.dumps(submitted, ensure_ascii=False).replace(key, "[REDACTED]"),
            flush=True,
        )
        task_id = submit_task_id(submitted)
    print(f"task_id={task_id}", flush=True)
    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        task = request_json(f"/mj/task/{task_id}/fetch", key)
        save_json(output / "task.json", task, key)
        status = task.get("status")
        print(
            json.dumps(
                {
                    "elapsed_seconds": round(time.monotonic() - started, 1),
                    "status": status,
                    "progress": task.get("progress"),
                    "failReason": task.get("failReason"),
                },
                ensure_ascii=False,
            ).replace(key, "[REDACTED]"),
            flush=True,
        )
        if status in TERMINAL_FAILURES:
            raise RuntimeError(f"task failed: {task.get('failReason')}")
        if status == "SUCCESS":
            image_url = task.get("imageUrl")
            if not isinstance(image_url, str) or not image_url.startswith("https://"):
                raise RuntimeError("successful task did not return an HTTPS imageUrl")
            # 第三方图片地址不携带上游 API key。
            with urllib.request.urlopen(image_url, timeout=90) as response:
                raw = response.read()
                mime = response.headers.get_content_type()
            if not mime.startswith("image/") or not raw:
                raise RuntimeError(f"image download returned {mime}, {len(raw)} bytes")
            image_path = output / "output.image"
            image_path.write_bytes(raw)
            summary = {
                "case": args.case,
                "requested_model": {
                    "blend": BLEND_MODEL,
                    "upscale": UPSCALE_MODEL,
                    "upscale-hd": UPSAMPLE_ACTION_MODEL,
                }.get(args.case, imagine_model),
                "requested_mode": args.mode,
                "task_id": task_id,
                "status": status,
                "elapsed_seconds": round(time.monotonic() - started, 2),
                "image_url": image_url,
                "download_mime": mime,
                "download_bytes": len(raw),
                "image_path": str(image_path),
            }
            save_json(output / "summary.json", summary, key)
            print(json.dumps(summary, ensure_ascii=False), flush=True)
            return
        time.sleep(min(POLL_SECONDS, max(0, deadline - time.monotonic())))
    raise TimeoutError(
        f"task {task_id} did not finish; resume with --task-id {task_id}"
    )


if __name__ == "__main__":
    main()
