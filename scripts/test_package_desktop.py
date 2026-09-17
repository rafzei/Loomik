"""Regression checks for the release dependency gate, without native build tools."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "package_desktop", Path(__file__).with_name("package-desktop.py")
)
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class DependencyChecks(unittest.TestCase):
    def mac_outputs(self, library, rpath="/usr/lib/swift"):
        return ["x86_64\n", f"loomik:\n\t{library} (compatibility version 1.0.0)\n",
                f"cmd LC_RPATH\npath {rpath} (offset 12)\n"]

    @patch.object(package.sys, "platform", "darwin")
    def test_intel_swift_overlay_resolves_from_os_runtime(self):
        with patch.object(package.subprocess, "check_output", side_effect=self.mac_outputs(
            "@rpath/libswiftAVFoundation.dylib"
        )), patch.object(package.ctypes, "CDLL") as load:
            package.verify_binary(Path("loomik"), "x86_64")
        load.assert_called_once_with("/usr/lib/swift/libswiftAVFoundation.dylib")

    @patch.object(package.sys, "platform", "darwin")
    def test_swift_overlay_requires_system_rpath(self):
        with patch.object(package.subprocess, "check_output", side_effect=self.mac_outputs(
            "@rpath/libswiftAVFoundation.dylib", "/Applications/Xcode.app/Contents/Developer/usr/lib/swift"
        )), self.assertRaisesRegex(RuntimeError, "Non-system dependency"):
            package.verify_binary(Path("loomik"), "x86_64")

    @patch.object(package.sys, "platform", "darwin")
    def test_swift_runtime_must_exist_on_host(self):
        with patch.object(package.subprocess, "check_output", side_effect=self.mac_outputs(
            "@rpath/libswiftMissing.dylib"
        )), patch.object(package.ctypes, "CDLL", side_effect=OSError("not found")), \
                self.assertRaisesRegex(RuntimeError, "Missing system Swift runtime"):
            package.verify_binary(Path("loomik"), "x86_64")

    @patch.object(package.sys, "platform", "darwin")
    def test_homebrew_dependency_remains_rejected(self):
        with patch.object(package.subprocess, "check_output", side_effect=self.mac_outputs(
            "/opt/homebrew/lib/libavcodec.dylib"
        )), self.assertRaisesRegex(RuntimeError, "Non-system dependency"):
            package.verify_binary(Path("ffmpeg"), "x86_64")

    @patch.object(package.sys, "platform", "win32")
    def test_windows_video_for_windows_system_imports(self):
        with patch.object(package.subprocess, "check_output", return_value=(
            "  Image has the following dependencies:\n\n"
            "    KERNEL32.dll\n    AVICAP32.dll\n    MSVFW32.dll\n"
        )):
            package.verify_binary(Path("ffmpeg.exe"), "x64")

    @patch.object(package.sys, "platform", "win32")
    def test_windows_native_capture_and_ui_system_imports(self):
        with patch.object(package.subprocess, "check_output", return_value=(
            "    bcryptPrimitives.dll\n    CoreMessaging.dll\n    MMDevAPI.dll\n"
            "    PSAPI.DLL\n    UxTheme.dll\n    api-ms-win-core-winrt-l1-1-0.dll\n"
        )):
            package.verify_binary(Path("loomik.exe"), "x64")

    @patch.object(package.sys, "platform", "win32")
    def test_external_windows_runtimes_remain_rejected(self):
        with patch.object(package.subprocess, "check_output", return_value=(
            "    KERNEL32.dll\n    VCRUNTIME140.dll\n    libgcc_s_seh-1.dll\n"
        )), self.assertRaisesRegex(RuntimeError, "libgcc_s_seh-1.dll, vcruntime140.dll"):
            package.verify_binary(Path("loomik.exe"), "x64")


if __name__ == "__main__":
    unittest.main()
