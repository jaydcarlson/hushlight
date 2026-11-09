import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from hushlight import Audio, Controller


class FakeAudio:
    def __init__(self):
        self.sinks = {
            "server:1": {"id": 10, "name": "speakers", "mute": False},
            "server:2": {"id": 20, "name": "headphones", "mute": True},
        }
        self.fail = False

    def snapshot(self):
        return copy.deepcopy(self.sinks)

    def mute(self, key, value):
        if self.fail:
            raise RuntimeError("Audio unavailable")
        self.sinks[key]["mute"] = value


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "state.json"
        self.audio = FakeAudio()
        self.controller = Controller(self.audio, self.path)

    def test_blank_wake_restores_each_original_state(self):
        before = self.audio.snapshot()
        self.controller.reconcile(True)
        self.assertTrue(all(s["mute"] for s in self.audio.sinks.values()))
        self.controller.reconcile(True)
        self.controller.reconcile(False)
        self.assertEqual(self.audio.sinks, before)
        self.assertEqual(self.controller.saved, {})

    def test_hotplug_and_manual_unmute_while_blank(self):
        self.controller.reconcile(True)
        self.audio.sinks["server:3"] = {"id": 30, "name": "HDMI", "mute": False}
        self.audio.sinks["server:1"]["mute"] = False
        self.controller.reconcile(True)
        self.assertTrue(all(s["mute"] for s in self.audio.sinks.values()))
        self.controller.reconcile(False)
        self.assertFalse(self.audio.sinks["server:3"]["mute"])

    def test_recover_after_crash_while_blank(self):
        before = self.audio.snapshot()
        self.controller.reconcile(True)
        recovered = Controller(self.audio, self.path)
        recovered.reconcile(True)
        recovered.reconcile(False)
        self.assertEqual(self.audio.sinks, before)

    def test_restore_retry_keeps_saved_state(self):
        self.controller.reconcile(True)
        self.audio.fail = True
        with self.assertRaises(RuntimeError):
            self.controller.reconcile(False)
        self.assertFalse(json.loads(self.path.read_text())["server:1"])
        self.audio.fail = False
        self.controller.reconcile(False)
        self.assertFalse(self.audio.sinks["server:1"]["mute"])

    def test_server_restart_cannot_restore_reused_ids(self):
        self.controller.reconcile(True)
        self.audio.sinks = {"newserver:1": {"id": 10, "name": "other", "mute": True}}
        self.controller.reconcile(False)
        self.assertTrue(self.audio.sinks["newserver:1"]["mute"])
        self.assertEqual(self.controller.saved, {})

    def test_empty_outputs_and_failed_mute_recover(self):
        self.audio.fail = True
        with self.assertRaises(RuntimeError):
            self.controller.reconcile(True)
        self.audio.fail = False
        self.controller.reconcile(True)
        self.controller.reconcile(False)
        self.assertFalse(self.audio.sinks["server:1"]["mute"])
        self.audio.sinks.clear()
        self.controller.reconcile(True)
        self.controller.reconcile(False)


class AudioTests(unittest.TestCase):
    @patch("hushlight.command")
    def test_snapshot_excludes_microphones(self, command):
        command.return_value = json.dumps([
            {"id": 0, "type": "PipeWire:Interface:Core", "info": {"cookie": 123}},
            {"id": 1, "type": "PipeWire:Interface:Node", "info": {
                "props": {"media.class": "Audio/Sink", "object.serial": 456},
                "params": {"Props": [{"mute": False}]}}},
            {"id": 2, "type": "PipeWire:Interface:Node", "info": {
                "props": {"media.class": "Audio/Source", "object.serial": 789}}},
        ])
        self.assertEqual(Audio().snapshot(), {
            "123:456": {"id": 1, "name": "1", "mute": False}})

    @patch("hushlight.command")
    def test_removed_output_is_not_mutated(self, command):
        audio = Audio()
        with patch.object(audio, "snapshot", return_value={}):
            audio.mute("old:1", False)
        command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
