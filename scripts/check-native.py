"""Inspect the real artifacts produced by --verify-recording. Requires FFmpeg."""
import array
import json
import pathlib
import struct
import subprocess
import sys

folder = pathlib.Path(sys.argv[1])
report = json.loads((folder / "native-result.json").read_text())
assert report["status"] == "recorded", report
assert report["pause_timer_verified"], "Timer advanced during pause"
assert report["countdown_digits"] == [3, 2, 1, 0], report
assert report["countdown_timer_zero"], "Media clock started during countdown"
assert report["began_after_zero"], "Recording started before zero was displayed"
assert report["cancelled_countdown_without_movie"], "Cancelled countdown saved a movie"
movie = report["output"]
metadata = json.loads(subprocess.check_output([
    "ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", movie
]))
video = next(s for s in metadata["streams"] if s["codec_type"] == "video")
assert video["codec_name"] == "h264"
assert 4.9 <= float(metadata["format"]["duration"]) <= 5.3
subprocess.run(["ffmpeg", "-v", "error", "-i", movie, "-f", "null", "-"], check=True)
audio = next((s for s in metadata["streams"] if s["codec_type"] == "audio"), None)
peak = None
if audio:
    assert audio["codec_name"] == "aac"
    assert abs(float(audio["duration"]) - float(video["duration"])) < 0.05
    raw = subprocess.check_output(["ffmpeg", "-v", "error", "-i", movie,
                                   "-vn", "-f", "f32le", "-ac", "1", "pipe:1"])
    samples = array.array("f", raw)
    if sys.byteorder != "little":
        samples.byteswap()
    peak = max(map(abs, samples), default=0)
png = pathlib.Path(report["screenshot"]).read_bytes()
assert png[:8] == b"\x89PNG\r\n\x1a\n"
dimensions = struct.unpack(">II", png[16:24])
assert dimensions == (video["width"], video["height"])
result = {
    "decode_ok": True,
    "pause_timer_verified": True,
    "video_codec": video["codec_name"],
    "dimensions": dimensions,
    "duration_seconds": float(metadata["format"]["duration"]),
    "frame_count": int(video["nb_frames"]),
    "audio_codec": audio["codec_name"] if audio else None,
    "audio_peak": peak,
    "audio_signal_detected": peak is not None and peak > 0.0001,
    "png_dimensions_match": True,
}
performance_path = pathlib.Path(movie).with_suffix(".performance.json")
if performance_path.exists():
    performance = json.loads(performance_path.read_text())
    assert video["has_b_frames"] == 0, "Encoder reordered frames"
    assert int(video["nb_frames"]) == performance["frames"]
    assert (video["width"], video["height"]) == (performance["width"], performance["height"])
    assert video["r_frame_rate"] == f'{performance["fps"]}/1'
    if report["camera"]:
        assert performance["missing_camera_frames"] == 0, "Camera frames were missing from the test"
    result["performance"] = performance
(folder / "media-check.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
