"""Offline-device selection regression tests; no Blender process/GPU required."""
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch


class DeviceSelectionTests(unittest.TestCase):
    def setUp(self):
        self.devices = [SimpleNamespace(type=kind, use=True)
                        for kind in ("CPU", "CUDA", "OPTIX")]
        self.preferences = SimpleNamespace(devices=self.devices, get_devices=lambda: None)
        self.scene = SimpleNamespace(cycles=SimpleNamespace(device="CPU"))
        bpy = SimpleNamespace(context=SimpleNamespace(preferences=SimpleNamespace(
            addons={"cycles": SimpleNamespace(preferences=self.preferences)})))
        self.mock = patch.dict(sys.modules, {"bpy": bpy})
        self.mock.start()
        sys.modules.pop("render_device", None)
        from render_device import configure_device
        self.configure = configure_device

    def tearDown(self):
        sys.modules.pop("render_device", None)
        self.mock.stop()

    def test_cpu_does_not_probe_or_change_preferences(self):
        self.preferences.get_devices = lambda: self.fail("CPU must not probe GPU devices")
        self.scene.cycles.device = "GPU"
        self.configure(self.scene, "CPU")
        self.assertEqual(self.scene.cycles.device, "CPU")
        self.assertTrue(all(device.use for device in self.devices))

    def test_gpu_enables_only_requested_backend(self):
        for backend in ("CUDA", "OPTIX"):
            with self.subTest(backend=backend):
                self.configure(self.scene, backend)
                self.assertEqual(self.scene.cycles.device, "GPU")
                self.assertEqual(self.preferences.compute_device_type, backend)
                self.assertEqual([device.type for device in self.devices if device.use], [backend])

    def test_missing_backend_fails_without_enabling_gpu(self):
        self.preferences.devices = self.devices[:1]
        with self.assertRaisesRegex(RuntimeError, "No OPTIX device"):
            self.configure(self.scene, "OPTIX")
        self.assertEqual(self.scene.cycles.device, "CPU")


if __name__ == "__main__":
    unittest.main()
