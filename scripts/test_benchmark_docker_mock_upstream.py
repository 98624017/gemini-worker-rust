import base64
import importlib.util
import subprocess
import sys
import tempfile
import unittest
import urllib.parse
from pathlib import Path
from unittest import mock


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

    def test_build_request_body_base64_mode_omits_output_flag(self) -> None:
        body = MODULE.build_request_body(
            [
                "https://img.example/1.png",
                "https://img.example/2.png",
                "https://img.example/3.png",
            ],
            output_mode="base64",
        )

        self.assertNotIn("output", body)

    def test_build_request_targets_includes_direct_and_proxy_generate_content_urls(
        self,
    ) -> None:
        targets = MODULE.build_request_targets(
            proxy_base_url="http://127.0.0.1:18788",
            direct_base_url="http://127.0.0.1:19090",
        )

        self.assertEqual(
            targets["proxy"],
            "http://127.0.0.1:18788/v1beta/models/bench:generateContent",
        )
        self.assertEqual(
            targets["direct"],
            "http://127.0.0.1:19090/v1beta/models/bench:generateContent",
        )

    def test_build_comparison_summary_reports_proxy_overhead(self) -> None:
        direct_results = [
            MODULE.RequestResult(1, 0.0, 110.0, 200, True, ""),
            MODULE.RequestResult(2, 1.0, 90.0, 200, True, ""),
            MODULE.RequestResult(3, 2.0, 130.0, 200, True, ""),
        ]
        proxy_results = [
            MODULE.RequestResult(1, 0.0, 240.0, 200, True, ""),
            MODULE.RequestResult(2, 1.0, 200.0, 200, True, ""),
            MODULE.RequestResult(3, 2.0, 260.0, 200, True, ""),
        ]

        summary = MODULE.build_comparison_summary(direct_results, proxy_results)

        self.assertEqual(summary["direct_total_ms"], 110.0)
        self.assertEqual(summary["proxy_total_ms"], 233.333)
        self.assertEqual(summary["proxy_overhead_ms"], 123.333)
        self.assertEqual(summary["direct_p50_ms"], 110.0)
        self.assertEqual(summary["proxy_p50_ms"], 240.0)
        self.assertEqual(summary["proxy_overhead_p50_ms"], 130.0)
        self.assertEqual(summary["direct_p95_ms"], 130.0)
        self.assertEqual(summary["proxy_p95_ms"], 260.0)
        self.assertEqual(summary["proxy_overhead_p95_ms"], 130.0)

    def test_build_scenario_metadata_marks_hit_and_miss(self) -> None:
        self.assertEqual(
            MODULE.build_scenario_metadata(output_mode="url", warm_cache=False),
            {
                "scenario": "miss/url",
                "cache_state": "miss",
                "output_mode": "url",
            },
        )
        self.assertEqual(
            MODULE.build_scenario_metadata(output_mode="base64", warm_cache=True),
            {
                "scenario": "hit/base64",
                "cache_state": "hit",
                "output_mode": "base64",
            },
        )

    def test_extract_stage_stats_row_reads_stage_fields(self) -> None:
        row = MODULE.extract_stage_stats_row(
            {
                "totalRequests": 2,
                "errorRequests": 0,
                "cacheHits": 1,
                "spillCount": 3,
                "spillBytesTotal": 4096,
                "requestParseMs": 12,
                "requestImagePrepareMs": 34,
                "requestImageMaterializeMs": 21,
                "requestImageFetchWorkMs": 17,
                "requestImageStoreWorkMs": 4,
                "requestEncodeMs": 13,
                "upstreamBuildMs": 56,
                "responseProcessMs": 78,
                "uploadMs": 90,
            },
            sampled_at=123.0,
        )

        self.assertEqual(
            row,
            {
                "timestamp": 123.0,
                "totalRequests": 2,
                "errorRequests": 0,
                "cacheHits": 1,
                "spillCount": 3,
                "spillBytesTotal": 4096,
                "requestParseMs": 12,
                "requestImagePrepareMs": 34,
                "requestImageMaterializeMs": 21,
                "requestImageFetchWorkMs": 17,
                "requestImageStoreWorkMs": 4,
                "requestEncodeMs": 13,
                "upstreamBuildMs": 56,
                "responseProcessMs": 78,
                "uploadMs": 90,
            },
        )

    def test_select_new_admin_log_items_filters_baseline_and_sorts(self) -> None:
        items = [
            {"id": 7, "durationMs": 30},
            {"id": 9, "durationMs": 50},
            {"id": 8, "durationMs": 40},
            {"id": "bad", "durationMs": 99},
        ]

        selected = MODULE.select_new_admin_log_items(items, baseline_log_id=7)

        self.assertEqual(selected, [{"id": 8, "durationMs": 40}, {"id": 9, "durationMs": 50}])

    def test_extract_admin_log_stage_row_reads_per_request_fields(self) -> None:
        row = MODULE.extract_admin_log_stage_row(
            {
                "id": 5,
                "statusCode": 200,
                "durationMs": 321,
                "requestParseMs": 12,
                "requestImagePrepareMs": 34,
                "requestImageMaterializeMs": 21,
                "requestImageFetchWorkMs": 17,
                "requestImageStoreWorkMs": 4,
                "requestEncodeMs": 13,
                "upstreamBuildMs": 56,
                "responseProcessMs": 78,
                "uploadMs": 90,
                "errorStage": "",
                "errorKind": "",
            }
        )

        self.assertEqual(
            row,
            {
                "id": 5,
                "statusCode": 200,
                "durationMs": 321,
                "requestParseMs": 12,
                "requestImagePrepareMs": 34,
                "requestImageMaterializeMs": 21,
                "requestImageFetchWorkMs": 17,
                "requestImageStoreWorkMs": 4,
                "requestEncodeMs": 13,
                "upstreamBuildMs": 56,
                "responseProcessMs": 78,
                "uploadMs": 90,
                "errorStage": "",
                "errorKind": "",
            },
        )

    def test_build_admin_log_stage_summary_reports_averages(self) -> None:
        summary = MODULE.build_admin_log_stage_summary(
            [
                {
                    "durationMs": 300,
                    "requestParseMs": 10,
                    "requestImagePrepareMs": 100,
                    "requestImageMaterializeMs": 90,
                    "requestImageFetchWorkMs": 240,
                    "requestImageStoreWorkMs": 5,
                    "requestEncodeMs": 10,
                    "upstreamBuildMs": 120,
                    "responseProcessMs": 20,
                    "uploadMs": 0,
                },
                {
                    "durationMs": 500,
                    "requestParseMs": 30,
                    "requestImagePrepareMs": 200,
                    "requestImageMaterializeMs": 150,
                    "requestImageFetchWorkMs": 360,
                    "requestImageStoreWorkMs": 15,
                    "requestEncodeMs": 20,
                    "upstreamBuildMs": 220,
                    "responseProcessMs": 40,
                    "uploadMs": 10,
                },
            ]
        )

        self.assertEqual(
            summary,
            {
                "avgDurationMs": 400.0,
                "avgRequestParseMs": 20.0,
                "avgRequestImagePrepareMs": 150.0,
                "avgRequestImageMaterializeMs": 120.0,
                "avgRequestImageFetchWorkMs": 300.0,
                "avgRequestImageStoreWorkMs": 10.0,
                "avgRequestEncodeMs": 15.0,
                "avgUpstreamBuildMs": 170.0,
                "avgResponseProcessMs": 30.0,
                "avgUploadMs": 5.0,
            },
        )

    def test_merge_admin_log_items_accumulates_and_dedupes_poll_results(self) -> None:
        items_by_id: dict[int, dict[str, object]] = {}
        max_seen_id = 7
        gap_detected = False

        max_seen_id, gap_detected = MODULE.merge_admin_log_items(
            items_by_id,
            [
                {"id": 8, "durationMs": 10},
                {"id": 9, "durationMs": 20},
            ],
            baseline_log_id=7,
            max_seen_id=max_seen_id,
            gap_detected=gap_detected,
        )
        max_seen_id, gap_detected = MODULE.merge_admin_log_items(
            items_by_id,
            [
                {"id": 9, "durationMs": 20},
                {"id": 10, "durationMs": 30},
            ],
            baseline_log_id=7,
            max_seen_id=max_seen_id,
            gap_detected=gap_detected,
        )

        self.assertEqual(max_seen_id, 10)
        self.assertFalse(gap_detected)
        self.assertEqual(
            sorted(items_by_id),
            [8, 9, 10],
        )

    def test_merge_admin_log_items_detects_gap_when_buffer_skips_ids(self) -> None:
        items_by_id: dict[int, dict[str, object]] = {}
        max_seen_id, gap_detected = MODULE.merge_admin_log_items(
            items_by_id,
            [
                {"id": 12, "durationMs": 20},
                {"id": 13, "durationMs": 30},
            ],
            baseline_log_id=7,
            max_seen_id=9,
            gap_detected=False,
        )

        self.assertEqual(max_seen_id, 13)
        self.assertTrue(gap_detected)
        self.assertEqual(sorted(items_by_id), [12, 13])

    def test_main_preserves_original_setup_error_instead_of_post_cleanup_crashing(
        self,
    ) -> None:
        class DummyServer:
            def shutdown(self) -> None:
                return

            def server_close(self) -> None:
                return

        argv = [
            "benchmark_docker_mock_upstream.py",
            "--image-url",
            "https://img.example/1.png",
            "--image-url",
            "https://img.example/2.png",
            "--image-url",
            "https://img.example/3.png",
            "--total-requests",
            "1",
            "--output-dir",
            tempfile.mkdtemp(),
        ]

        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            MODULE,
            "start_mock_server",
            return_value=(DummyServer(), 19090),
        ), mock.patch.object(
            MODULE,
            "run_command",
            return_value=subprocess.CompletedProcess(
                args=["docker"], returncode=1, stdout="", stderr="docker failed"
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "docker failed"):
                MODULE.main()

    def test_append_cache_buster_adds_request_id(self) -> None:
        self.assertEqual(
            MODULE.append_cache_buster("https://img.example/a.png", 7),
            "https://img.example/a.png?bench_request_id=7",
        )

    def test_append_cache_buster_preserves_existing_query(self) -> None:
        rewritten = MODULE.append_cache_buster(
            "https://img.example/a.png?size=10mb",
            9,
        )
        parsed = urllib.parse.urlparse(rewritten)
        query = urllib.parse.parse_qs(parsed.query)

        self.assertEqual(parsed.scheme, "https")
        self.assertEqual(parsed.netloc, "img.example")
        self.assertEqual(parsed.path, "/a.png")
        self.assertEqual(query["size"], ["10mb"])
        self.assertEqual(query["bench_request_id"], ["9"])


if __name__ == "__main__":
    unittest.main()
