import base64
import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("benchmark_docker_mock_upstream.py")
SPEC = importlib.util.spec_from_file_location(
    "benchmark_docker_mock_upstream", SCRIPT_PATH
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class BenchmarkHelperTest(unittest.TestCase):
    def test_build_base64_payload_hits_target_size(self) -> None:
        payload = MODULE.build_base64_payload(20 * 1024 * 1024)
        self.assertEqual(len(payload), 20 * 1024 * 1024)
        decoded = base64.b64decode(payload, validate=True)
        self.assertGreater(len(decoded), 0)

    def test_build_request_body_contains_three_image_urls(self) -> None:
        body = MODULE.build_request_body(
            [
                "https://img.example/1.png",
                "https://img.example/2.png",
                "https://img.example/3.png",
            ]
        )
        self.assertEqual(body["output"], "url")
        self.assertEqual(len(body["contents"][0]["parts"]), 3)
        self.assertEqual(
            body["contents"][0]["parts"][2]["inlineData"]["data"],
            "https://img.example/3.png",
        )


if __name__ == "__main__":
    unittest.main()
