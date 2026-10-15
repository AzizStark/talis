"""Choose an offline Cycles bake device without saving Blender preferences."""
import bpy


def configure_device(scene, backend):
    if backend == "CPU":
        scene.cycles.device = "CPU"
        return
    preferences = bpy.context.preferences.addons["cycles"].preferences
    preferences.compute_device_type = backend
    preferences.get_devices()
    devices = [device for device in preferences.devices if device.type == backend]
    if not devices:
        raise RuntimeError(f"No {backend} device available; retry with --device CPU")
    for device in preferences.devices:
        device.use = device.type == backend
    scene.cycles.device = "GPU"

