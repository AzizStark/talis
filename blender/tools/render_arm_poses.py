"""Bake cropped fixed-camera Cycles arm poses without saving the source."""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Matrix, Vector
sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from render_resolution import pixel_grid

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
parser.add_argument("--frames", type=int, default=33)
parser.add_argument("--samples", type=int, default=2048)
parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="CPU")
parser.add_argument("--inspect", action="store_true")
parser.add_argument("--crop", help="Verified pixel crop x,y,width,height; defaults to conservative light bounds")
add_arguments(parser)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
if not 2 <= args.frames <= 128 or args.samples < 1:
    parser.error("Use 2..128 poses and a positive sample count")
scene = bpy.context.scene
source = Path(bpy.data.filepath)
source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
beauty = args.output.parent / "beauty.png"
reference = json.loads(beauty.with_suffix(".json").read_text())
if reference["source_sha256"] != source_hash:
    raise RuntimeError("Re-render the beauty from this source before preparing arm poses")
width, height = reference["resolution"]
camera = scene.camera
if camera is None:
    raise RuntimeError("An active camera is required")
fixed = {"Tonearm pedestal - revised", "Tonearm pedestal shoulder", "Tonearm vertical bearing"}
moving = [o for o in scene.objects if o.parent and o.parent.name == "Turntable - move this assembly"
          and o.name.startswith(("Tonearm", "Headshell", "Cartridge", "Stylus")) and o.name not in fixed]
if not moving:
    raise RuntimeError("No separate tonearm objects found")
pivot = bpy.data.objects["Tonearm horizontal bearing barrel"].matrix_world.translation.copy()
tip = bpy.data.objects["Stylus diamond contact tip"].matrix_world.translation.copy()
record = bpy.data.objects["Vinyl record - 12 inch"]
center = record.matrix_world.translation.copy()
radius = record.dimensions.x / 2
start_radius, end_radius = radius * 0.96, radius * 0.40
d = Vector((pivot.x - center.x, pivot.y - center.y))
u = Vector((tip.x - pivot.x, tip.y - pivot.y))

def wrapped(angle):
    return (angle + math.pi) % (2 * math.pi) - math.pi

def angle_for_radius(target):
    cosine = (target * target - d.length_squared - u.length_squared) / (2 * d.length * u.length)
    if not -1 <= cosine <= 1:
        raise RuntimeError("Requested groove radius is outside this arm's reach")
    heading = math.atan2(d.y, d.x)
    tip_heading = math.atan2(u.y, u.x)
    solutions = [wrapped(heading + sign * math.acos(cosine) - tip_heading) for sign in (-1, 1)]
    return min(solutions, key=abs)

start_angle, end_angle = angle_for_radius(start_radius), angle_for_radius(end_radius)
original = {o.name: o.matrix_world.copy() for o in moving}
bounds = [o.matrix_world @ Vector(corner) for o in moving for corner in o.bound_box]

def transform(angle):
    return Matrix.Translation(pivot) @ Matrix.Rotation(angle, 4, "Z") @ Matrix.Translation(-pivot)

# Conservative coverage: moving geometry, sun shadows and planar mirror projections.
# Include all potential receivers and margin for area-light softness / rough reflections.
lights = [o for o in scene.objects if o.type == "LIGHT" and not o.hide_render
          and o.data.energy > 0]
directional = [o.matrix_world.to_quaternion() @ Vector((0, 0, -1))
               for o in lights if o.data.type == "SUN"]
emitters = []
for light in lights:
    if light.data.type == "SUN":
        continue
    emitters.append(light.matrix_world.translation.copy())
    if light.data.type == "AREA":
        half_x = light.data.size / 2
        half_y = getattr(light.data, "size_y", light.data.size) / 2
        if light.data.shape != "RECTANGLE":
            half_y = half_x
        emitters.extend(light.matrix_world @ Vector((x, y, 0))
                        for x in (-half_x, half_x) for y in (-half_y, half_y))
def world_bounds(name):
    o = bpy.data.objects[name]
    return [o.matrix_world @ Vector(corner) for corner in o.bound_box]

wall_y = min(p.y for p in world_bounds("Wall - cream plaster"))
receiver_z = [max(p.z for p in world_bounds(name)) for name in
              ["Tabletop - amber wood", "Inset brushed silver deck", "Vinyl record - 12 inch"]]
projected = []
for index in range(65):
    matrix = transform(start_angle + (end_angle - start_angle) * index / 64)
    for point in bounds:
        p = matrix @ point
        candidates = [p]
        for z in receiver_z:
            candidates.append(Vector((p.x, p.y, 2 * z - p.z)))
        # Point/area-lit night scenes need bounds too; do not assume an active sun.
        for ray in [*directional, *(p - emitter for emitter in emitters)]:
            for z in receiver_z:
                if abs(ray.z) > 1e-8:
                    t = (z - p.z) / ray.z
                    if t >= 0:
                        candidates.append(p + ray * t)
            if abs(ray.y) > 1e-8:
                t = (wall_y - p.y) / ray.y
                if t >= 0:
                    candidates.append(p + ray * t)
        for candidate in candidates:
            v = world_to_camera_view(scene, camera, candidate)
            if v.z > 0:
                projected.append((v.x * width, (1 - v.y) * height))
# Quantize to exactly representable border fractions, avoiding one-pixel crop shifts.
grid_x, grid_y = pixel_grid(width, height)
left = max(0, int(math.floor((min(p[0] for p in projected) - 64) / grid_x) * grid_x))
right = min(width, int(math.ceil((max(p[0] for p in projected) + 64) / grid_x) * grid_x))
top = max(0, int(math.floor((min(p[1] for p in projected) - 64) / grid_y) * grid_y))
bottom = min(height, int(math.ceil((max(p[1] for p in projected) + 64) / grid_y) * grid_y))
if args.crop:
    try:
        values = [int(v) for v in args.crop.split(",")]
    except ValueError:
        parser.error("Crop requires four integer pixel coordinates x,y,width,height")
    if len(values) != 4:
        parser.error("Crop requires x,y,width,height")
    x, y, w, h = values
    if (min(x,y) < 0 or min(w,h) < 1 or x+w > width or y+h > height
            or any(abs(v/grid-round(v/grid)) > 1e-6 for v,grid in
                   [(x,grid_x),(x+w,grid_x),(y,grid_y),(y+h,grid_y)])):
        parser.error("Crop must be positive, within the image and aligned to its exact integer border grid")
    left, top, right, bottom = x, y, x+w, y+h
frames = [{"file": f"pose-{i:03}.png", "progress": i / (args.frames - 1),
           "angle_degrees": math.degrees(start_angle + (end_angle - start_angle) * i / (args.frames - 1))}
          for i in range(args.frames)]
metadata = {
    "format_version": 1, "source": str(source), "source_sha256": source_hash,
    "base_image_sha256": hashlib.sha256(beauty.read_bytes()).hexdigest(),
    "width": width, "height": height, "crop": [left, top, right - left, bottom - top],
    "frames": frames, "pivot_blender": list(pivot), "moving_objects": [o.name for o in moving],
    "start_radius": start_radius, "end_radius": end_radius,
    "start_angle_degrees": math.degrees(start_angle), "end_angle_degrees": math.degrees(end_angle),
    "camera": camera.name, "samples": args.samples,
    "crop_method": "Verified override" if args.crop else "Conservative active-light bounds",
    "limitations": ["Fixed-camera/light/finish 2.5D pose test, not runtime 3D arm rendering",
                    "Nearest-pose playback is sharp but stepped between discrete samples",
                    "Cropped complete renders include shadows/reflections; indirect changes beyond the crop are not updated"],
}
print("TALIS_ARM_PLAN " + json.dumps(metadata), flush=True)
if not args.inspect:
    args.output.mkdir(parents=True, exist_ok=True)
    scene.render.engine = "CYCLES"
    configure_device(scene, args.device)
    configure_quality(scene, args)
    scene.render.use_persistent_data = True
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.render.use_border = True
    scene.render.use_crop_to_border = True
    scene.render.border_min_x, scene.render.border_max_x = left / width, right / width
    scene.render.border_min_y, scene.render.border_max_y = 1 - bottom / height, 1 - top / height
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = args.color_depth
    metadata["quality"] = quality_metadata(scene)
    for i, frame in enumerate(frames):
        matrix = transform(math.radians(frame["angle_degrees"]))
        for o in moving:
            o.matrix_world = matrix @ original[o.name]
        bpy.context.view_layer.update()
        scene.render.filepath = str((args.output / frame["file"]).resolve())
        print(f"TALIS_ARM_POSE {i + 1}/{args.frames} progress={frame['progress']:.3f}", flush=True)
        bpy.ops.render.render(write_still=True)
        # The original authoring scene is never saved, only generated PNGs/metadata.
    if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
        raise RuntimeError("Source scene changed during pose rendering")
    metadata["denoising"] = True
    metadata["blender"] = bpy.app.version_string
    metadata["compute_backend"] = args.device
    (args.output / "atlas.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print("TALIS_ARM_COMPLETE; original .blend unchanged", flush=True)
