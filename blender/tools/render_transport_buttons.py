"""Bake real button travel into tiny camera-aligned patches; never save the scene."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from render_resolution import button_crop, validate_resolution

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--author-manifest", required=True)
parser.add_argument("--variant", choices=("day", "night"), required=True)
parser.add_argument("--output", required=True)
parser.add_argument("--steps", type=int, default=8)
parser.add_argument("--width", type=int, default=2560)
parser.add_argument("--height", type=int, default=1440)
parser.add_argument("--samples", type=int, default=2048)
parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="OPTIX")
add_arguments(parser)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
validate_resolution(args.width,args.height)
if not 4 <= args.steps <= 16:
    raise ValueError("Use 4..16 intermediate travel steps")
author = json.loads(Path(args.author_manifest).read_text())
info = author["modes"][args.variant]
source = Path(bpy.data.filepath)
source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
if source_hash != info["scene_sha256"]:
    raise ValueError("Button scene does not match the authored manifest")
scene = bpy.context.scene
if scene.camera.name != info["camera"]:
    raise ValueError("Button bake requires the authored fixed camera")
out = Path(args.output)
out.mkdir(parents=True, exist_ok=True)
scene.frame_set(1)
controller = bpy.data.objects[author["controller"]]
if controller.animation_data:
    controller.animation_data.action = None
keys = ("Play pause pressed", "Previous pressed", "Next pressed")
for key in keys:
    controller[key] = 0.0
controller.update_tag()
bpy.context.view_layer.update()
scene.render.engine = "CYCLES"
configure_device(scene, args.device)
configure_quality(scene, args)
scene.render.use_persistent_data = True
width, height = args.width, args.height
scene.render.resolution_x, scene.render.resolution_y = width, height
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"
scene.render.film_transparent = False
scene.use_nodes = False
boxes = list(info["patch_rectangles_scene_uv"].values())
# Shared crop captures contact shadows/reflections as well as the actual caps.
# Quantization keeps Blender's float border rounding exact at this resolution.
left, top, crop_width, crop_height = button_crop(width, height, boxes)
right, bottom = left + crop_width, top + crop_height
scene.render.use_border = scene.render.use_crop_to_border = True
scene.render.border_min_x, scene.render.border_max_x = left / width, right / width
scene.render.border_min_y, scene.render.border_max_y = 1 - bottom / height, 1 - top / height
files = []

def render(name):
    scene.render.filepath = str(out / name)
    bpy.ops.render.render(write_still=True)
    files.append(name)
    print("BUTTON_POSE_BAKED", args.variant, name, flush=True)

render("released.png")
for index, key in enumerate(keys):
    for step in range(1, args.steps + 1):
        controller[key] = step / args.steps
        controller.update_tag()
        bpy.context.view_layer.update()
        render(f"button-{index}-{step:02}.png")
    controller[key] = 0.0
    controller.update_tag()
    bpy.context.view_layer.update()
if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
    raise RuntimeError("Source changed while rendering; do not package this bake")
metadata = {
    "format_version": 1, "width": width, "height": height,
    "crop": [left, top, right-left, bottom-top], "steps": args.steps,
    "order": ["play_pause", "previous", "next"], "renders": files,
    "source": str(source), "source_sha256": source_hash,
    "quality": quality_metadata(scene), "variant": args.variant,
    "encoding": "Blender display-encoded RGBA8; no runtime tonemapping",
}
(out / "bake.json").write_text(json.dumps(metadata, indent=2) + "\n")
