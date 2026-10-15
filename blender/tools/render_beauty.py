"""Render a faithful fixed-camera baseline without saving the input .blend."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata


parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
parser.add_argument("--width", type=int, default=2560)
parser.add_argument("--height", type=int, default=1440)
parser.add_argument("--samples", type=int, default=2048)
parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="CPU")
add_arguments(parser)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
if min(args.width, args.height, args.samples) < 1:
    parser.error("Resolution and sample count must be positive")

scene = bpy.context.scene
if scene.camera is None:
    raise RuntimeError("The source scene needs an active camera")
source = Path(bpy.data.filepath)
source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
scene.render.engine = "CYCLES"
# Portable offline preparation: no runtime ray tracing or GPU access is required.
configure_device(scene, args.device)
configure_quality(scene, args)
scene.render.resolution_x = args.width
scene.render.resolution_y = args.height
scene.render.resolution_percentage = 100
scene.render.use_border = False
scene.render.use_crop_to_border = False
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGB"
scene.render.image_settings.color_depth = args.color_depth
scene.render.filepath = str(args.output.resolve())
args.output.parent.mkdir(parents=True, exist_ok=True)
metadata = {
    "purpose": "Complete static Cycles beauty baseline; dynamic layers are not separated",
    "source": str(source),
    "source_sha256": source_hash,
    "blender": bpy.app.version_string,
    "engine": scene.render.engine,
    "device": scene.cycles.device,
    "compute_backend": args.device,
    "resolution": [args.width, args.height],
    "samples": args.samples,
    "denoising": scene.cycles.use_denoising,
    "camera": scene.camera.name,
    "frame": scene.frame_current,
    "view_transform": scene.view_settings.view_transform,
    "look": scene.view_settings.look,
    "exposure": scene.view_settings.exposure,
    "gamma": scene.view_settings.gamma,
    "display_device": scene.display_settings.display_device,
    "quality": quality_metadata(scene),
}
if "talis_fill_provenance" in scene:
    metadata["lighting_variant"] = json.loads(scene["talis_fill_provenance"])
print("Talis beauty render: " + json.dumps(metadata), flush=True)
bpy.ops.render.render(write_still=True)
metadata["image_sha256"] = hashlib.sha256(args.output.read_bytes()).hexdigest()
args.output.with_suffix(".json").write_text(json.dumps(metadata, indent=2) + "\n")
if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
    raise RuntimeError("Source scene changed during rendering")
print("Talis beauty render complete; original scene unchanged", flush=True)
