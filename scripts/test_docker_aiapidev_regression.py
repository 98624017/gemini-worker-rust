import importlib.util
import json
import sys
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("docker_aiapidev_regression.py")
SPEC = importlib.util.spec_from_file_location("docker_aiapidev_regression", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class DockerAiapidevRegressionTest(unittest.TestCase):
    def test_default_request_timeout_seconds_is_450(self) -> None:
        self.assertEqual(MODULE.DEFAULT_REQUEST_TIMEOUT_SECONDS, 450.0)

    def test_build_aiapidev_request_body_url_mode(self) -> None:
        body = MODULE.build_aiapidev_request_body(
            image_urls=[
                "https://img.example/1.png",
                "https://img.example/2.jpg",
            ],
            output_mode="url",
        )

        self.assertEqual(
            body["generationConfig"]["imageConfig"]["output"],
            "url",
        )
        self.assertEqual(body["contents"][0]["role"], "user")
        self.assertEqual(len(body["contents"][0]["parts"]), 3)
        self.assertEqual(
            body["contents"][0]["parts"][2]["inlineData"]["data"],
            "https://img.example/2.jpg",
        )

    def test_build_aiapidev_request_body_base64_mode_omits_output(self) -> None:
        body = MODULE.build_aiapidev_request_body(
            image_urls=[
                "https://img.example/1.png",
                "https://img.example/2.jpg",
            ],
            output_mode=None,
        )

        self.assertNotIn("output", body["generationConfig"]["imageConfig"])
        self.assertEqual(
            body["generationConfig"]["responseModalities"],
            ["IMAGE"],
        )

    def test_validate_url_mode_response_extracts_result(self) -> None:
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
                          "data": "https://pub.example/result.png"
                        }
                      }
                    ]
                  },
                  "finishReason": "STOP"
                }
              ],
              "usageMetadata": {
                "promptTokenCount": 1024,
                "candidatesTokenCount": 1024,
                "totalTokenCount": 2048
              }
            }
            """
        )

        summary = MODULE.validate_url_mode_response(body)
        self.assertEqual(summary["image_url"], "https://pub.example/result.png")
        self.assertEqual(summary["usage_total"], 2048)

    def test_validate_base64_mode_response_extracts_metadata(self) -> None:
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
                          "data": "QUJDREVGRw=="
                        }
                      }
                    ]
                  },
                  "finishReason": "STOP"
                }
              ],
              "usageMetadata": {
                "promptTokenCount": 1024,
                "candidatesTokenCount": 1024,
                "totalTokenCount": 2048
              }
            }
            """
        )

        summary = MODULE.validate_base64_mode_response(body)
        self.assertEqual(summary["mime_type"], "image/png")
        self.assertEqual(summary["data_len"], 12)
        self.assertEqual(summary["usage_total"], 2048)

    def test_validate_failure_response_extracts_message(self) -> None:
        body = {
            "error": {
                "code": 502,
                "message": "failed to fetch image url: 404",
            }
        }

        summary = MODULE.validate_failure_response(502, body)
        self.assertEqual(summary["status_code"], 502)
        self.assertIn("404", summary["message"])


if __name__ == "__main__":
    unittest.main()
