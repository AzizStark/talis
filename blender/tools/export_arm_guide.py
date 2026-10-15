"""Export a fixed-camera steel-tube motion guide; never render or save the scene."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
from mathutils import Vector


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--poses", type=Path, default=Path("assets/scenes/day/arm-poses/atlas.json"))
    parser.add_argument("--output", type=Path, default=Path("assets/scenes/day/arm-guide.json"))
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    encoded = args.poses.read_bytes()
    poses = json.loads(encoded)
    source = Path(bpy.data.filepath)
    if args.output.resolve() in (source.resolve(), args.poses.resolve()):
        raise ValueError("Guide output must not overwrite the Blender source or pose manifest")
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    if source_hash != poses["source_sha256"]:
        raise RuntimeError("The guide's Blender scene must match the authored pose package")
    scene = bpy.context.scene
    if scene.camera is None or scene.camera.name != poses["camera"]:
        raise RuntimeError("The guide must use the original pose camera")
    name = "Tonearm - level tube with controlled end bend"
    obj = bpy.data.objects[name]
    if name not in poses["moving_objects"]:
        raise RuntimeError("The tube is not part of this pose sweep")
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh()
    try:
        mesh.calc_loop_triangles()
        vertices = [list(evaluated.matrix_world @ v.co) for v in mesh.vertices]
        triangles = [list(t.vertices) for t in mesh.loop_triangles]
    finally:
        evaluated.to_mesh_clear()
    if not vertices or not triangles:
        raise RuntimeError("The tube has no evaluated geometry")
    low, high = min(v[2] for v in vertices), max(v[2] for v in vertices)
    width, height = poses["width"], poses["height"]
    left, top, _, _ = poses["crop"]
    camera = scene.camera
    clip = camera.calc_matrix_camera(
        bpy.context.evaluated_depsgraph_get(), x=width, y=height,
        scale_x=scene.render.pixel_aspect_x, scale_y=scene.render.pixel_aspect_y,
    ) @ camera.matrix_world.inverted()
    # Pixel coordinates at boundaries: cell centers are (x+.5, y+.5).
    projection = [list((clip[0] + clip[3]) * (width / 2) - clip[3] * left),
                  list((clip[3] - clip[1]) * (height / 2) - clip[3] * top),
                  list(clip[3])]
    pivot = bpy.data.objects["Tonearm horizontal bearing barrel"].matrix_world.translation
    if (pivot - Vector(poses["pivot_blender"])).length > 1e-6:
        raise RuntimeError("The guide pivot does not match the pose package")
    guide = {
        "format_version": 1, "object": name,
        "pose_manifest_sha256": hashlib.sha256(encoded).hexdigest(),
        "source_sha256": source_hash, "projection": projection,
        "vertices": vertices, "triangles": triangles,
        "plane_z": (low + high) / 2,
        "limitations": ["Tube motion uses a horizontal center-plane approximation",
                        "Guide masks are projected geometry, not Cycles visibility passes"],
    }
    # All moving parts protect their RGB from background-edge correction, even
    # where a headshell/counterweight crosses the tube's projected neighborhood.
    foreground_vertices, foreground_triangles = [], []
    for object_name in poses["moving_objects"]:
        moving = bpy.data.objects[object_name].evaluated_get(bpy.context.evaluated_depsgraph_get())
        moving_mesh = moving.to_mesh()
        if moving_mesh is None:
            continue
        try:
            moving_mesh.calc_loop_triangles()
            offset = len(foreground_vertices)
            foreground_vertices.extend(list(moving.matrix_world @ v.co) for v in moving_mesh.vertices)
            foreground_triangles.extend([offset+i for i in t.vertices] for t in moving_mesh.loop_triangles)
        finally:
            moving.to_mesh_clear()
    guide["foreground_vertices"] = foreground_vertices
    guide["foreground_triangles"] = foreground_triangles
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(guide, separators=(",", ":")) + "\n")
    if hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
        raise RuntimeError("The original scene changed")
    print(f"TALIS_ARM_GUIDE {len(vertices)} vertices, {len(triangles)} triangles; "
          f"tube z={low:.6f}..{high:.6f}; original scene unchanged", flush=True)


if __name__ == "__main__":
    main()
