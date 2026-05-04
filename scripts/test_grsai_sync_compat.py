import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("grsai_sync_compat.py")
SPEC = importlib.util.spec_from_file_location("grsai_sync_compat", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class GrsaiSyncCompatTest(unittest.TestCase):
    def test_expected_grsai_request_body_matches_provider_contract(self) -> None:
        self.assertEqual(
            MODULE.expected_grsai_request_body(
                model="nano-banana-fast",
                prompt="两张图片合并",
                urls=[
                    "https://img.example/1.png",
                    "https://img.example/2.png",
                ],
                aspect_ratio="16:9",
                image_size="2K",
            ),
            {
                "model": "nano-banana-fast",
                "prompt": "两张图片合并",
                "urls": [
                    "https://img.example/1.png",
                    "https://img.example/2.png",
                ],
                "aspectRatio": "16:9",
                "imageSize": "2K",
                "shutProgress": True,
            },
        )

    def test_summarize_gemini_success_ignores_non_contract_fields(self) -> None:
        body = json.loads(
            """
            {
              "candidates": [
                {
                  "content": {
                    "role": "model",
                    "parts": [
                      {
                        "inlineData": {
                          "mimeType": "image/png",
                          "data": "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fapi.grsai.com%2Fimg%2F1.png"
                        }
                      }
                    ]
                  },
                  "finishReason": "STOP",
                  "safetyRatings": []
                }
              ],
              "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 1,
                "totalTokenCount": 2
              },
              "bananaProxyMeta": {
                "durationMs": 123
              }
            }
            """
        )

        self.assertEqual(
            MODULE.summarize_gemini_success(body),
            {
                "mime_type": "image/png",
                "data": "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fapi.grsai.com%2Fimg%2F1.png",
            },
        )

    def test_summarize_openai_success_extracts_first_image_url(self) -> None:
        body = json.loads(
            """
            {
              "created": 1714800000,
              "data": [
                {
                  "url": "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fapi.grsai.com%2Fimg%2F1.png"
                }
              ],
              "usage": {
                "total_tokens": 2048
              },
              "upstream_meta": {
                "status": "succeeded"
              }
            }
            """
        )

        self.assertEqual(
            MODULE.summarize_openai_success(body),
            {
                "image_url": "https://proxy.example.com/proxy/image?url=https%3A%2F%2Fapi.grsai.com%2Fimg%2F1.png",
            },
        )

    def test_summarize_error_response_keeps_only_shared_semantics(self) -> None:
        go_body = {
            "error": {
                "code": 401,
                "message": "Upstream service temporarily unavailable.",
                "status": "UNAUTHENTICATED",
            }
        }
        rust_body = {
            "error": {
                "code": 401,
                "message": "上游服务鉴权失败，请检查密钥后再试",
                "source": "proxy",
                "stage": "parse_upstream_response",
                "kind": "upstream_auth_failed",
            }
        }

        self.assertEqual(
            MODULE.summarize_error_response(401, go_body),
            {"status_code": 401, "error_code": 401},
        )
        self.assertEqual(
            MODULE.summarize_error_response(401, rust_body),
            {"status_code": 401, "error_code": 401},
        )


if __name__ == "__main__":
    unittest.main()
