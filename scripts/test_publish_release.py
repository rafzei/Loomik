import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import publish_release as release


class SourceRunTests(unittest.TestCase):
    def setUp(self):
        self.run = {"repository": {"full_name": release.REPO},
                    "head_repository": {"full_name": release.REPO},
                    "event": "push", "head_branch": "main",
                    "path": ".github/workflows/portable-builds.yml",
                    "status": "completed", "conclusion": "success",
                    "head_sha": "a" * 40, "run_attempt": 1}
        self.jobs = {"total_count": 4, "jobs": [
            {"name": f"Build/package {platform} (runner {runner})", "conclusion": "success"}
            for platform, (runner, _, _) in release.PLATFORMS.items()]}

    def test_accepts_successful_main_build(self):
        with patch.object(release, "api", side_effect=[self.run, {"status": "ahead"}, self.jobs]):
            self.assertEqual(release.validate_run("123")["head_sha"], "a" * 40)

    def test_rejects_untrusted_or_failed_builds(self):
        changes = [{"head_repository": {"full_name": "outsider/Loomik"}},
                   {"repository": {"full_name": "outsider/Loomik"}},
                   {"event": "pull_request"}, {"event": "workflow_dispatch"},
                   {"head_branch": "feature"}, {"path": ".github/workflows/other.yml"},
                   {"status": "in_progress"}, {"conclusion": "failure"}]
        for change in changes:
            with self.subTest(change=change), patch.object(release, "api", return_value=self.run | change):
                with self.assertRaises(ValueError):
                    release.validate_run("123")

    def test_rejects_commit_outside_main(self):
        with patch.object(release, "api", side_effect=[self.run, {"status": "diverged"}]):
            with self.assertRaisesRegex(ValueError, "belong to main"):
                release.validate_run("123")

    def test_rejects_skipped_missing_or_unrelated_jobs(self):
        for kind in ("skipped", "missing", "unrelated"):
            jobs = copy.deepcopy(self.jobs)
            if kind == "skipped":
                jobs["jobs"][0]["conclusion"] = "skipped"
            elif kind == "missing":
                jobs["jobs"].pop()
            else:
                jobs["jobs"][0]["name"] = "Unrelated job"
            with self.subTest(kind=kind), patch.object(
                    release, "api", side_effect=[self.run, {"status": "identical"}, jobs]):
                with self.assertRaisesRegex(ValueError, "four native"):
                    release.validate_run("123")

    def test_rejects_invalid_run_id_before_api(self):
        for run_id in ("../123", "1; echo bad", "0", "-1", "12\n13"):
            with self.subTest(run_id=run_id), patch.object(release, "api") as api:
                with self.assertRaises(ValueError):
                    release.validate_run(run_id)
                api.assert_not_called()


class PackageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.packages, _ = release.asset_names("1.0.0")
        for name in self.packages:
            (self.root / name).write_bytes(name.encode())
        for platform, (_, suffix, extensions) in release.PLATFORMS.items():
            stem = f"Loomik-1.0.0-{suffix}"
            entries = [{"name": f"{stem}.{ext}",
                        "sha256": release.sha256(self.root / f"{stem}.{ext}"),
                        "bytes": (self.root / f"{stem}.{ext}").stat().st_size}
                       for ext in extensions]
            checksum_name = stem + (".deb.sha256" if extensions == ("deb",) else ".sha256")
            (self.root / checksum_name).write_text("".join(
                f"{item['sha256']}  {item['name']}\n" for item in entries))
            if extensions != ("deb",):
                report = {"version": "1.0.0", "platform": platform, "files": entries,
                          "package_check": {"status": "passed", "checks": [
                              {"format": ext, "decoded_frames": 30, "audio_peak": 0.1}
                              for ext in ("mp4", "mov", "mkv")]}}
                (self.root / f"{stem}.json").write_text(json.dumps(report))

    def test_verifies_complete_package_set(self):
        files, checksums = release.verify_packages(self.root, "1.0.0")
        self.assertEqual(len(files), 13)
        self.assertEqual(set(checksums), self.packages)

    def test_rejects_corrupted_package(self):
        (self.root / sorted(self.packages)[0]).write_bytes(b"corrupted")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            release.verify_packages(self.root, "1.0.0")

    def test_rejects_duplicate_asset_names(self):
        directory = self.root / "other-artifact"
        directory.mkdir()
        (directory / sorted(self.packages)[0]).write_bytes(b"duplicate")
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            release.verify_packages(self.root, "1.0.0")

    def test_rejects_checksum_path_traversal(self):
        next(self.root.glob("*.sha256")).write_text("a" * 64 + "  ../secret\n")
        with self.assertRaisesRegex(ValueError, "Invalid checksum entry"):
            release.verify_packages(self.root, "1.0.0")

    def test_rejects_extra_files(self):
        (self.root / "unexpected.exe").write_bytes(b"unexpected")
        with self.assertRaisesRegex(ValueError, "Unexpected artifact"):
            release.verify_packages(self.root, "1.0.0")

    def test_rejects_incomplete_media_checks(self):
        path = next(self.root.glob("*.json"))
        report = json.loads(path.read_text())
        report["package_check"]["checks"].pop()
        path.write_text(json.dumps(report))
        with self.assertRaisesRegex(ValueError, "Incomplete media"):
            release.verify_packages(self.root, "1.0.0")


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.expected = {"package.zip": {"size": 123, "digest": "sha256:" + "a" * 64}}
        self.asset = {"name": "package.zip", "state": "uploaded", **self.expected["package.zip"]}

    def test_identical_uploaded_assets_are_idempotent(self):
        self.assertEqual(release.verify_uploaded({"assets": [self.asset]}, self.expected), set())

    def test_refuses_to_overwrite_different_or_incomplete_assets(self):
        for change in ({"digest": "sha256:" + "b" * 64}, {"size": 1}, {"state": "starter"}):
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "refusing to overwrite"):
                release.verify_uploaded({"assets": [self.asset | change]}, self.expected,
                                        allow_missing=True)

    def test_missing_assets_allowed_only_for_draft_recovery(self):
        with self.assertRaisesRegex(ValueError, "incomplete"):
            release.verify_uploaded({"assets": []}, self.expected)
        self.assertEqual(release.verify_uploaded({"assets": []}, self.expected, allow_missing=True),
                         {"package.zip"})

    def test_rejects_wrong_tag_commit(self):
        with patch.object(release, "api", return_value={"object": {"type": "commit", "sha": "b" * 40}}):
            with self.assertRaisesRegex(ValueError, "different commit"):
                release.verify_tag("v1.0.0", "a" * 40)

    def test_resolves_annotated_tag(self):
        with patch.object(release, "api", side_effect=[
                {"object": {"type": "tag", "sha": "b" * 40}},
                {"object": {"type": "commit", "sha": "a" * 40}}]):
            release.verify_tag("v1.0.0", "a" * 40)


if __name__ == "__main__":
    unittest.main()
