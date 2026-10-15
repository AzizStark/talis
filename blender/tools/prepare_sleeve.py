"""Bake a fixed-camera printed sleeve response; never save the authoring scene.

White/black camera-albedo calibration separates diffuse illumination from the
paper's additive sheen/coat. Secondary-ray albedo stays at the authored paper
colour, freezing colour bounce consistently with the cached room. At an AA edge
the residual also retains the uncovered background: do not multiply coverage twice.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from render_resolution import pixel_grid


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pixels(path):
    image = bpy.data.images.load(str(path), check_existing=False)
    image.colorspace_settings.name = "Non-Color"
    data = np.empty(image.size[0] * image.size[1] * 4, np.float32)
    image.pixels.foreach_get(data)
    data = data.reshape(image.size[1], image.size[0], 4)[::-1].copy()
    bpy.data.images.remove(image)
    return data


def bake_lut(scene, work, edge, low, high):
    # The PNG is only a temporary transport of Blender's own view transform.
    # Texel order for OpenGL is red fastest, then green, then blue.
    values = np.exp2(np.linspace(low, high, edge, dtype=np.float32))
    blue, green, red = np.meshgrid(values, values, values, indexing="ij")
    rgba = np.stack((red, green, blue, np.ones_like(red)), axis=-1)
    rows = rgba.reshape(edge * edge, edge, 4)
    image = bpy.data.images.new("Talis sleeve colour LUT", width=edge,
                                height=edge * edge, float_buffer=True)
    image.pixels.foreach_set(rows[::-1].copy().ravel())
    image.update()
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.image_settings.color_depth = "8"
    scene.render.dither_intensity = 0
    path = work / "view-lut.png"
    image.save_render(str(path), scene=scene)
    bpy.data.images.remove(image)
    encoded = np.rint(np.clip(pixels(path), 0, 1) * 255).astype(np.uint8)
    return encoded.reshape(edge, edge, edge, 4)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--beauty", type=Path, default=Path("assets/scenes/day/beauty.png"))
    parser.add_argument("--samples", type=int, default=2048)
    parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="CPU")
    parser.add_argument("--lut-edge", type=int, default=96)
    parser.add_argument("--object", default="Placeholder sleeve - top printed board",
                        help="Printed board to calibrate; only its upward-facing material is changed")
    parser.add_argument("--response-kind", choices=("sleeve", "stack"), default="sleeve")
    parser.add_argument("--reference-cover", type=Path, action="append", default=[])
    parser.add_argument("--source-reference", type=Path,
                        help="Current-source beauty JSON: prove sleeve-region compatibility with an older base")
    add_arguments(parser)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    if args.samples < 1 or not 16 <= args.lut_edge <= 128:
        parser.error("Positive samples and a LUT edge between 16 and 128 are required")
    scene = bpy.context.scene
    source = Path(bpy.data.filepath)
    source_hash = digest(source)
    beauty_hash = digest(args.beauty)
    baseline = json.loads(args.beauty.with_suffix(".json").read_text())
    if baseline["image_sha256"] != beauty_hash:
        raise ValueError("The current beauty sidecar/image hashes differ")
    if baseline["source_sha256"] != source_hash and args.source_reference is None:
        raise ValueError("Source differs from the beauty: re-render it or provide a verified --source-reference")
    if scene.use_nodes:
        raise ValueError("Custom compositing needs explicit support before baking this scene")
    obj = bpy.data.objects[args.object]
    top = max(v.co.z for v in obj.data.vertices)
    xs = [v.co.x for v in obj.data.vertices]
    ys = [v.co.y for v in obj.data.vertices]
    local_corners = [(min(xs), max(ys), top), (max(xs), max(ys), top),
                     (max(xs), min(ys), top), (min(xs), min(ys), top)]
    corners = []
    for point in local_corners:
        v = world_to_camera_view(scene, scene.camera, obj.matrix_world @ Vector(point))
        corners.append([v.x, 1 - v.y])
    width, height = baseline["resolution"]
    gx, gy = pixel_grid(width, height)
    left = max(0, int(math.floor((min(p[0] for p in corners)*width - 8)/gx)*gx))
    right = min(width, int(math.ceil((max(p[0] for p in corners)*width + 8)/gx)*gx))
    upper = max(0, int(math.floor((min(p[1] for p in corners)*height - 8)/gy)*gy))
    bottom = min(height, int(math.ceil((max(p[1] for p in corners)*height + 8)/gy)*gy))
    crop = [left, upper, right-left, bottom-upper]
    compatibility = None
    if baseline["source_sha256"] != source_hash:
        proof = json.loads(args.source_reference.read_text())
        proof_image = args.source_reference.with_suffix(".png")
        keys = ["resolution", "camera", "frame", "view_transform", "look", "exposure", "gamma", "display_device"]
        if (proof["source_sha256"] != source_hash or proof["image_sha256"] != digest(proof_image)
                or any(proof[key] != baseline[key] for key in keys)):
            raise ValueError("Source-reference camera/colour settings/hash do not match")
        a, b = pixels(args.beauty), pixels(proof_image)
        if a.shape != b.shape:
            raise ValueError("Source-reference dimensions differ")
        error = np.abs(a[upper:bottom,left:right,:3] - b[upper:bottom,left:right,:3])*255
        if error.max() > 4.1 or error.mean() > .01:
            raise ValueError(f"Sleeve region changed: max={error.max()}, mean={error.mean()}; re-render the room")
        compatibility = {"reference_sha256": digest(proof_image),
                         "max_error_255": float(error.max()), "mean_error_255": float(error.mean())}
        print("TALIS_SLEEVE_REGION_MATCH " + json.dumps(compatibility), flush=True)
    front_slots = {p.material_index for p in obj.data.polygons if p.normal.z > .99}
    if len(front_slots) != 1:
        raise ValueError("Expected one dedicated sleeve front material slot")
    slot = front_slots.pop()
    if any(p.material_index == slot and p.normal.z < .99 for p in obj.data.polygons):
        raise ValueError("Front material also covers the folded edges")
    material = obj.data.materials[slot].copy()
    obj.data.materials[slot] = material
    nodes, links = material.node_tree.nodes, material.node_tree.links
    principled = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
    base = principled.inputs["Base Color"]
    if principled.inputs["Metallic"].default_value != 0:
        raise ValueError("Calibration requires nonmetallic paper")
    if base.is_linked and args.response_kind != "stack":
        raise ValueError("Existing sleeve calibration requires constant base colour")
    original_link = base.links[0].from_socket if base.is_linked else None
    original_colour = list(base.default_value)
    path = nodes.new("ShaderNodeLightPath")
    mix = nodes.new("ShaderNodeMixRGB")
    mix.inputs[1].default_value = original_colour
    if original_link is not None:
        # Keep the authored print for secondary rays. Only camera albedo changes;
        # room bounce/reflections remain compatible with the existing beauty.
        links.new(original_link, mix.inputs[1])
    links.new(path.outputs["Is Camera Ray"], mix.inputs[0])
    links.new(mix.outputs[0], base)
    layer = bpy.context.view_layer
    layer.use_pass_cryptomatte_object = False
    aov = layer.aovs.add()
    aov.name, aov.type = "TalisSleeveCoverage", "VALUE"
    output_aov = nodes.new("ShaderNodeOutputAOV")
    output_aov.name = aov.name
    output_aov.inputs["Value"].default_value = 1
    args.output.mkdir(parents=True, exist_ok=True)
    work = args.output / "work"
    work.mkdir(exist_ok=True)
    scene.render.engine = "CYCLES"
    configure_device(scene, args.device)
    configure_quality(scene, args)
    quality = quality_metadata(scene)
    scene.render.use_persistent_data = True
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.use_crop_to_border = True
    scene.render.border_min_x, scene.render.border_max_x = left/width, right/width
    scene.render.border_min_y, scene.render.border_max_y = 1-bottom/height, 1-upper/height
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = args.color_depth
    scene.use_nodes = True
    tree = scene.node_tree
    tree.nodes.clear()
    render = tree.nodes.new("CompositorNodeRLayers")
    combined = tree.nodes.new("CompositorNodeComposite")
    tree.links.new(render.outputs["Image"], combined.inputs["Image"])
    output = tree.nodes.new("CompositorNodeOutputFile")
    output.base_path = str(work.resolve())
    output.format.file_format = "OPEN_EXR"
    output.format.color_mode, output.format.color_depth = "RGBA", "32"
    output.file_slots[0].path = "linear-"
    tree.links.new(render.outputs["Image"], output.inputs[0])
    output.file_slots.new("coverage-")
    tree.links.new(render.outputs[aov.name], output.inputs[1])

    def render_reference(label, colour):
        mix.inputs[2].default_value = colour
        output.file_slots[0].path = label + "-linear-"
        output.file_slots[1].path = label + "-coverage-"
        scene.render.filepath = str((work / (label + ".png")).resolve())
        print("TALIS_SLEEVE_RENDER " + label, flush=True)
        bpy.ops.render.render(write_still=True)
        return pixels(work / f"{label}-linear-{scene.frame_current:04}.exr")

    white = render_reference("white", (1, 1, 1, 1))
    mask = pixels(work / f"white-coverage-{scene.frame_current:04}.exr")[..., 0]
    black = render_reference("black", (0, 0, 0, 1))
    if white.shape[:2] != (crop[3], crop[2]) or black.shape != white.shape:
        raise ValueError("Blender crop dimensions differ from the declared crop")
    illumination = np.maximum(white[..., :3] - black[..., :3], 0)
    residual = np.maximum(black[..., :3], 0)
    if not np.isfinite(white).all() or not np.isfinite(black).all() or not np.isfinite(mask).all():
        raise ValueError("Nonfinite baked lighting")
    maps = {"illumination.rgba16f": np.dstack((illumination, np.clip(mask, 0, 1))),
            "residual.rgba16f": np.dstack((residual, np.ones_like(mask)))}
    low, high = -16.0, max(6.0, math.ceil(math.log2(max(1, float(white[..., :3].max())))) + 1)
    lut = bake_lut(scene, work, args.lut_edge, low, high)
    files = {}
    for name, data in maps.items():
        half = data.astype("<f2")
        if not np.isfinite(half).all():
            raise ValueError("Lighting exceeds half-float range")
        target = args.output / name
        target.write_bytes(half.tobytes())
        files[name] = {"file": name, "sha256": digest(target)}
    lut_path = args.output / "view.rgba8"
    lut_path.write_bytes(lut.tobytes())
    # Reference covers are development outputs, never runtime assets.
    tex = nodes.new("ShaderNodeTexImage")
    coords = nodes.new("ShaderNodeTexCoord")
    tex.extension = "EXTEND"
    links.new(coords.outputs["Generated"], tex.inputs["Vector"])
    links.new(tex.outputs["Color"], mix.inputs[2])
    for cover in args.reference_cover:
        tex.image = bpy.data.images.load(str(cover.resolve()), check_existing=False)
        render_reference("reference-" + cover.stem, (1, 1, 1, 1))
    if digest(source) != source_hash or digest(args.beauty) != beauty_hash:
        raise ValueError("Source scene/beauty changed during baking")
    metadata = {"format_version": 1, "source": str(source), "source_sha256": source_hash,
                "response_kind": args.response_kind, "object": args.object,
                "base_image_sha256": beauty_hash, "width": width, "height": height,
                "crop": crop, "corners": corners, "samples": args.samples,
                "blender": bpy.app.version_string, "compute_backend": args.device,
                "quality": quality,
                "illumination": files["illumination.rgba16f"],
                "residual": files["residual.rgba16f"],
                "view_lut": {"file": lut_path.name, "sha256": digest(lut_path)},
                "lut_edge": args.lut_edge, "lut_log_min": low, "lut_log_max": high,
                "view_transform": scene.view_settings.view_transform,
                "look": scene.view_settings.look, "exposure": scene.view_settings.exposure,
                "gamma": scene.view_settings.gamma,
                "base_source_sha256": baseline["source_sha256"], "region_compatibility": compatibility,
                "limitations": ["Fixed camera, lighting and paper material",
                    "Printed colour does not change secondary-ray bounce or reflections elsewhere",
                    "AA coverage is already included in illumination/residual; do not multiply it twice"]}
    (args.output / "atlas.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print("TALIS_SLEEVE_COMPLETE " + json.dumps({"crop": crop, "corners": corners,
        "coverage_max": float(mask.max()), "lighting_max": float(white[..., :3].max())}), flush=True)


if __name__ == "__main__":
    main()
