"""Bake the authored cover-OFF state against an exact beauty; never save the blend."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--beauty", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=2048)
    parser.add_argument("--device", choices=("CPU", "CUDA", "OPTIX"), default="OPTIX")
    add_arguments(parser)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    source = Path(bpy.data.filepath)
    source_hash, base_hash = digest(source), digest(args.beauty)
    baseline = json.loads(args.beauty.with_suffix(".json").read_text())
    if baseline["source_sha256"] != source_hash or baseline["image_sha256"] != base_hash:
        raise ValueError("Empty stand must use the exact source of the current beauty")
    scene = bpy.context.scene
    if scene.camera.name != baseline["camera"] or scene.use_nodes:
        raise ValueError("Camera/custom compositing differs from the baseline")
    scene.frame_set(baseline["frame"])
    controller = bpy.data.objects["Placeholder LP sleeve - cream"]
    stand = bpy.data.objects["Sleeve stand - continuous rounded metal frame"]
    parts = [obj for obj in bpy.data.objects if obj.name.startswith("Placeholder sleeve - ")]
    if len(parts) != 7 or "Cover visible" not in controller:
        raise ValueError("Expected the authored cover switch and seven sleeve meshes")
    controller["Cover visible"] = False
    controller.update_tag()
    bpy.context.view_layer.update()
    if not all(obj.hide_render for obj in parts) or stand.hide_render:
        raise ValueError("Cover-OFF must hide every sleeve mesh and retain the steel stand")
    colour = {
        "view_transform": scene.view_settings.view_transform,
        "look": scene.view_settings.look,
        "exposure": scene.view_settings.exposure,
        "gamma": scene.view_settings.gamma,
        "display_device": scene.display_settings.display_device,
    }
    if any(colour[key] != baseline[key] for key in colour):
        raise ValueError("Empty-stand colour settings differ from the baseline")
    scene.render.engine = "CYCLES"
    configure_device(scene, args.device)
    configure_quality(scene, args)
    width, height = baseline["resolution"]
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.use_crop_to_border = False
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = args.color_depth
    args.output.mkdir(parents=True, exist_ok=True)
    target = args.output / "empty.png"
    scene.render.filepath = str(target.resolve())
    print("TALIS_EMPTY_STAND_RENDER " + json.dumps({"source": str(source), "quality": quality_metadata(scene)}), flush=True)
    bpy.ops.render.render(write_still=True)
    if digest(source) != source_hash or digest(args.beauty) != base_hash:
        raise ValueError("Source/beauty changed while baking")
    manifest = {
        "format_version": 1, "width": width, "height": height,
        "base_image_sha256": base_hash, "source_sha256": source_hash,
        "source": str(source), "image": {"file": target.name, "sha256": digest(target)},
        "controller": controller.name, "cover_visible": False,
        "hidden_parts": sorted(obj.name for obj in parts), "preserved_stand": stand.name,
        "frame": scene.frame_current, "camera": scene.camera.name,
        "blender": bpy.app.version_string, "compute_backend": args.device,
        "quality": quality_metadata(scene), **colour,
    }
    (args.output / "atlas.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("TALIS_EMPTY_STAND_COMPLETE " + str(args.output), flush=True)


if __name__ == "__main__":
    main()
