"""Bake stationary paper lighting for a rotating label; never save the source.

The temporary camera-only paper plane covers the maximum record-artwork fit,
allowing the existing size control to remain independent of the lighting.
Secondary rays see the original record. Moving arm geometry is excluded;
fixed spindle visibility and environmental lighting remain in the response.
"""
import argparse
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from bpy_extras.object_utils import world_to_camera_view

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_sleeve import bake_lut, digest, pixels
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from render_resolution import pixel_grid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--beauty", type=Path, required=True)
    parser.add_argument("--poses", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=2048)
    parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="CPU")
    parser.add_argument("--lut-edge", type=int, default=96)
    parser.add_argument("--reference-cover", type=Path, action="append", default=[])
    parser.add_argument("--reference-size", type=float, default=35.0)
    parser.add_argument("--reference-corners", type=float, nargs=8,
                        default=[.38203880, .49312621, .57033771, .49312621,
                                 .58017546, .62102577, .36554030, .62102577],
                        help="Runtime artwork quad for independent reference renders, TL/TR/BR/BL")
    add_arguments(parser)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    if not 16 <= args.lut_edge <= 128 or not 5 <= args.reference_size <= 100:
        parser.error("Use LUT edge 16..128 and reference size 5..100")
    source = Path(bpy.data.filepath)
    source_hash, beauty_hash = digest(source), digest(args.beauty)
    baseline = json.loads(args.beauty.with_suffix(".json").read_text())
    poses = json.loads(args.poses.read_text())
    if (baseline["source_sha256"] != source_hash or baseline["image_sha256"] != beauty_hash
            or poses["source_sha256"] != source_hash or poses["base_image_sha256"] != beauty_hash):
        raise ValueError("Source, beauty and arm package must match before calibrating the label")
    scene = bpy.context.scene
    if scene.use_nodes or scene.camera.name != baseline["camera"]:
        raise ValueError("Expected the matched authored camera and no custom compositor")
    record = bpy.data.objects["Vinyl record - 12 inch"]
    original_label = bpy.data.objects["Paper center label"]
    original_label.visible_camera = False
    # Old label decorations should not become stationary ink on fetched artwork.
    for obj in scene.objects:
        if "label" in obj.name.lower() and obj is not original_label:
            # Camera invisibility alone still lets raised ink/borders cast their
            # silhouettes onto the temporary paper. Exclude them from all rays.
            obj.hide_render = True
    for name in poses["moving_objects"]:
        bpy.data.objects[name].hide_render = True
    center = record.matrix_world.translation
    radius = record.dimensions.x / 2
    z = max((original_label.matrix_world @ Vector(c)).z for c in original_label.bound_box) + 0.00001
    points = [Vector((center.x + x*radius, center.y + y*radius, z))
              for x, y in [(-1, 1), (1, 1), (1, -1), (-1, -1)]]
    mesh = bpy.data.meshes.new("Talis temporary maximum-size label paper")
    # Positive world-Z face normal; explicit UVs keep the artwork upright.
    mesh.from_pydata(points, [], [(3, 2, 1, 0)])
    paper = bpy.data.objects.new("Talis camera-only label calibration", mesh)
    scene.collection.objects.link(paper)
    uv = mesh.uv_layers.new(name="Label plane UV")
    source_uv = [(0, 1), (1, 1), (1, 0), (0, 0)]
    for loop in mesh.loops:
        uv.data[loop.index].uv = source_uv[loop.vertex_index]
    material = original_label.data.materials[0].copy()
    mesh.materials.append(material)
    nodes, links = material.node_tree.nodes, material.node_tree.links
    principled = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
    base = principled.inputs["Base Color"]
    if principled.inputs["Metallic"].is_linked or principled.inputs["Metallic"].default_value != 0:
        raise ValueError("Expected nonmetallic label paper")
    if base.is_linked:
        # The authored label now multiplies its ink colour by a fixed spindle
        # contact-shading response. Calibrate the ink input, not the output:
        # disconnecting Base Color would erase that stationary shading.
        tint = base.links[0].from_node
        if (tint.type != "MIX_RGB" or tint.blend_type != "MULTIPLY"
                or tint.inputs[0].is_linked or tint.inputs[0].default_value != 1
                or tint.inputs[1].is_linked):
            raise ValueError("Unsupported linked label colour; expected constant ink multiplied by contact shading")
        base = tint.inputs[1]
        # Keep radial contact shading in the original label's coordinate space
        # rather than the larger temporary calibration plane's local space.
        for node in nodes:
            if node.type == "TEX_COORD" and node.object is None:
                node.object = original_label
    output_material = next(n for n in nodes if n.type == "OUTPUT_MATERIAL")
    transparent = nodes.new("ShaderNodeBsdfTransparent")
    path = nodes.new("ShaderNodeLightPath")
    shader_mix = nodes.new("ShaderNodeMixShader")
    links.new(path.outputs["Is Camera Ray"], shader_mix.inputs[0])
    links.new(transparent.outputs[0], shader_mix.inputs[1])
    links.new(principled.outputs[0], shader_mix.inputs[2])
    links.new(shader_mix.outputs[0], output_material.inputs["Surface"])
    layer = bpy.context.view_layer
    aov = layer.aovs.add()
    aov.name, aov.type = "TalisRecordPaperCoverage", "VALUE"
    aov_output = nodes.new("ShaderNodeOutputAOV")
    aov_output.name = aov.name
    aov_output.inputs["Value"].default_value = 1
    bpy.context.view_layer.update()
    corners = []
    for point in points:
        projected = world_to_camera_view(scene, scene.camera, point)
        corners.append([projected.x, 1-projected.y])
    width, height = baseline["resolution"]
    gx, gy = pixel_grid(width, height)
    left = max(0, int(math.floor((min(p[0] for p in corners)*width-12)/gx)*gx))
    right = min(width, int(math.ceil((max(p[0] for p in corners)*width+12)/gx)*gx))
    top = max(0, int(math.floor((min(p[1] for p in corners)*height-12)/gy)*gy))
    bottom = min(height, int(math.ceil((max(p[1] for p in corners)*height+12)/gy)*gy))
    crop = [left, top, right-left, bottom-top]
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
    scene.render.border_min_y, scene.render.border_max_y = 1-bottom/height, 1-top/height
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.use_nodes = True
    tree = scene.node_tree
    tree.nodes.clear()
    render = tree.nodes.new("CompositorNodeRLayers")
    composite = tree.nodes.new("CompositorNodeComposite")
    tree.links.new(render.outputs["Image"], composite.inputs["Image"])
    output = tree.nodes.new("CompositorNodeOutputFile")
    output.base_path = str(work.resolve())
    output.format.file_format = "OPEN_EXR"
    output.format.color_mode, output.format.color_depth = "RGBA", "32"
    output.file_slots[0].path = "linear-"
    tree.links.new(render.outputs["Image"], output.inputs[0])
    output.file_slots.new("coverage-")
    tree.links.new(render.outputs[aov.name], output.inputs[1])

    def render_reference(name, colour):
        base.default_value = colour
        output.file_slots[0].path = name + "-linear-"
        output.file_slots[1].path = name + "-coverage-"
        scene.render.filepath = str((work / (name + ".png")).resolve())
        print("TALIS_RECORD_RENDER " + name, flush=True)
        bpy.ops.render.render(write_still=True)
        return pixels(work / f"{name}-linear-{scene.frame_current:04}.exr")

    white = render_reference("white", (1, 1, 1, 1))
    mask = pixels(work / f"white-coverage-{scene.frame_current:04}.exr")[..., 0]
    black = render_reference("black", (0, 0, 0, 1))
    if white.shape != black.shape or white.shape[:2] != (crop[3], crop[2]):
        raise ValueError("Record crop dimensions differ")
    if not all(np.isfinite(a).all() for a in (white, black, mask)):
        raise ValueError("Nonfinite label calibration")
    maps = {"illumination.rgba16f": np.dstack((np.maximum(white[...,:3]-black[...,:3], 0), np.clip(mask, 0, 1))),
            "residual.rgba16f": np.dstack((np.maximum(black[...,:3], 0), np.ones_like(mask)))}
    files = {}
    for name, data in maps.items():
        half = data.astype("<f2")
        if not np.isfinite(half).all():
            raise ValueError("Label lighting exceeds half-float range")
        target = args.output/name
        target.write_bytes(half.tobytes())
        files[name] = {"file": name, "sha256": digest(target)}
    low, high = -16.0, max(6.0, math.ceil(math.log2(max(1, float(white[...,:3].max()))))+1)
    lut = bake_lut(scene, work, args.lut_edge, low, high)
    lut_path = args.output/"view.rgba8"
    lut_path.write_bytes(lut.tobytes())
    # Independent reference renders use a smaller paper patch matching the
    # runtime homography rather than resizing/moving the already baked lighting.
    if args.reference_cover:
        uv_corners = np.array([[0, 0], [1, 0], [1, 1], [0, 1]], dtype=float)
        screen = np.array(args.reference_corners, dtype=float).reshape(4, 2)
        rows, target = [], []
        for (u, v), (x, y) in zip(uv_corners, screen):
            rows += [[u, v, 1, 0, 0, 0, -x*u, -x*v], [0, 0, 0, u, v, 1, -y*u, -y*v]]
            target += [x, y]
        h = np.append(np.linalg.solve(rows, target), 1).reshape(3, 3)
        inverse_clip = (scene.camera.calc_matrix_camera(bpy.context.evaluated_depsgraph_get(),
                        x=width, y=height, scale_x=scene.render.pixel_aspect_x,
                        scale_y=scene.render.pixel_aspect_y) @ scene.camera.matrix_world.inverted()).inverted()
        for vertex, (u, v) in zip(mesh.vertices, uv_corners):
            scale = args.reference_size/100
            q = h @ np.array([.5+(u-.5)*scale, .5+(v-.5)*scale, 1])
            x, y = q[:2]/q[2]
            near = inverse_clip @ Vector((2*x-1, 1-2*y, -1, 1))
            far = inverse_clip @ Vector((2*x-1, 1-2*y, 1, 1))
            origin, end = near.to_3d()/near.w, far.to_3d()/far.w
            ray = end-origin
            vertex.co = origin+ray*((z-origin.z)/ray.z)
        mesh.update()
        bpy.context.view_layer.update()
        tex = nodes.new("ShaderNodeTexImage")
        coords = nodes.new("ShaderNodeTexCoord")
        tex.extension = "EXTEND"
        links.new(coords.outputs["UV"], tex.inputs["Vector"])
        links.new(tex.outputs["Color"], base)
        for cover in args.reference_cover:
            tex.image = bpy.data.images.load(str(cover.resolve()), check_existing=False)
            render_reference("reference-"+cover.stem, (1, 1, 1, 1))
    if digest(source) != source_hash or digest(args.beauty) != beauty_hash:
        raise ValueError("Source/beauty changed during label baking")
    metadata = {"format_version": 1, "response_kind": "record_lighting", "source": str(source), "source_sha256": source_hash,
                "base_image_sha256": beauty_hash, "width": width, "height": height,
                "crop": crop, "corners": corners, "samples": args.samples,
                "blender": bpy.app.version_string, "compute_backend": args.device, "quality": quality,
                "illumination": files["illumination.rgba16f"], "residual": files["residual.rgba16f"],
                "view_lut": {"file": lut_path.name, "sha256": digest(lut_path)},
                "lut_edge": args.lut_edge, "lut_log_min": low, "lut_log_max": high,
                "view_transform": scene.view_settings.view_transform, "look": scene.view_settings.look,
                "exposure": scene.view_settings.exposure, "gamma": scene.view_settings.gamma,
                "reference_size": args.reference_size, "excluded_moving_objects": poses["moving_objects"],
                "limitations": ["Fixed scene-space lighting; only fetched artwork rotates",
                    "Moving arm is excluded: no live or frozen arm lighting/shadow on the artwork",
                    "Original scene supplies secondary-ray bounce; artwork does not relight the room",
                    "Response spans maximum label fit; circle size remains a runtime control",
                    "Calibration coverage includes fixed spindle occlusion and AA"]}
    (args.output/"atlas.json").write_text(json.dumps(metadata, indent=2)+"\n")
    print("TALIS_RECORD_COMPLETE " + json.dumps({"crop": crop, "corners": corners,
          "coverage_max": float(mask.max())}), flush=True)


if __name__ == "__main__":
    main()
