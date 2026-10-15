"""Verify a complete high-quality day OR night bake before publishing it.

Standard-library provenance/quality checks complement the app's runtime loaders
and native framebuffer tests. Run separately for each lighting package.
"""
import argparse
import hashlib
import json
import math
import struct
from pathlib import Path
from scene_package_contract import reject_legacy_center_refresh, validate_base_bindings


def require(condition, message):
    if not condition:
        raise ValueError(message)


def valid_knob_crop(crop, dimensions):
    x, y, w, h = crop
    # Match the runtime: allow padded 4K rectangles, but retain the original
    # 256x256 total pixel ceiling and all scene/axis bounds.
    return (min(x,y) >= 0 and 0 < w <= 512 and 0 < h <= 512
            and w*h <= 256*256 and x+w <= dimensions[0] and y+h <= dimensions[1])


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest(path):
    return json.loads(path.read_text())


def local_file(root, name):
    require(isinstance(name, str) and Path(name).name == name and name not in ("", ".", ".."),
            "Manifest filename must be local to its package")
    return root / name


def check_png(path, dimensions, depth):
    with path.open("rb") as file:
        header = file.read(33)
    require(header[:8] == b"\x89PNG\r\n\x1a\n" and header[12:16] == b"IHDR",
            f"Invalid PNG header: {path}")
    require(struct.unpack(">II", header[16:24]) == tuple(dimensions),
            f"Wrong PNG dimensions: {path}")
    require(header[24] == depth, f"Wrong PNG bit depth: {path}")


def check_quality(data, args):
    expected = {"samples": args.samples, "use_adaptive_sampling": True,
                "adaptive_threshold": args.noise_threshold,
                "adaptive_min_samples": args.min_samples, "use_denoising": True,
                "denoiser": "OPENIMAGEDENOISE", "denoising_input_passes": "RGB_ALBEDO_NORMAL",
                "denoising_prefilter": "ACCURATE", "seed": 0,
                "use_animated_seed": False, "png_color_depth": "16"}
    require(data["samples"] == args.samples, "Sample maximum differs from expected quality")
    quality = data.get("quality", {})
    require(all(quality.get("view_layer_denoising", {}).values()),
            "A rendered view layer disables denoising")
    for key, value in expected.items():
        actual = quality.get(key)
        same = (isinstance(actual, (float, int)) and math.isclose(actual, value, rel_tol=1e-7)
                if key == "adaptive_threshold" else actual == value)
        require(same, f"Unexpected quality setting {key}: {actual!r}")


def verify(args):
    root = args.scenes
    beauty = manifest(root / "beauty.json")
    reject_legacy_center_refresh(beauty)
    beauty_hash = digest(root / "beauty.png")
    validate_base_bindings(root, beauty_hash)
    poses = manifest(root / "arm-poses/atlas.json")
    guide = manifest(root / "arm-guide.json")
    flow = manifest(root / "arm-flow/atlas.json")
    sleeve = manifest(root / "sleeve/atlas.json")
    source_hash = beauty["source_sha256"]
    knob_path = root / "knob/atlas.json"
    if knob_path.exists():
        knob = manifest(knob_path)
        require(knob["source_sha256"] == source_hash, "Knob calibration uses another source")
        require(knob["base_image_sha256"] == beauty_hash, "Knob uses another beauty")
        require([knob["width"], knob["height"]] == beauty["resolution"], "Knob scene dimensions differ")
        check_quality({"samples": knob["quality"]["samples"], "quality": knob["quality"]}, args)
        x, y, w, h = knob["crop"]
        require(valid_knob_crop(knob["crop"], beauty["resolution"]), "Invalid knob crop")
        for key in ("clean", "ink"):
            path = local_file(knob_path.parent, knob[key]["file"])
            require(digest(path) == knob[key]["sha256"], "Corrupt knob " + key)
            check_png(path, (w, h), 16)
        print(f"PASS HQ knob: {knob_path}; {w}x{h}, matched stationary body/marker response")
    require(args.source is None or digest(args.source) == source_hash, "Source snapshot hash differs")
    require(beauty["image_sha256"] == beauty_hash, "Beauty image hash differs")
    dimensions = beauty["resolution"]
    stand_path = root / "empty-stand/atlas.json"
    if stand_path.exists():
        stand = manifest(stand_path)
        require(stand["format_version"] == 1 and stand["source_sha256"] == source_hash
                and stand["base_image_sha256"] == beauty_hash
                and [stand["width"], stand["height"]] == dimensions
                and stand["cover_visible"] is False, "Stale/invalid empty stand")
        require(stand["camera"] == beauty["camera"] and stand["frame"] == beauty["frame"],
                "Empty-stand framing differs")
        for key in ("view_transform", "look", "exposure", "gamma"):
            require(stand[key] == beauty[key], "Empty-stand colour settings differ")
        check_quality({"samples": stand["quality"]["samples"], "quality": stand["quality"]}, args)
        path = local_file(stand_path.parent, stand["image"]["file"])
        require(digest(path) == stand["image"]["sha256"], "Corrupt empty stand")
        check_png(path, dimensions, 16)
        print(f"PASS HQ empty stand: {stand_path}")
    region_path = root / "arm-poses/update-region.json"
    if region_path.exists():
        region = manifest(region_path)
        require(region["format_version"] == 1
                and region["pose_manifest_sha256"] == digest(root / "arm-poses/atlas.json")
                and region["flow_manifest_sha256"] == digest(root / "arm-flow/atlas.json"), "Stale arm display ROI")
        x,y,w,h = region["crop"]
        bx,by,bw,bh = poses["crop"]
        require(x >= bx and y >= by and w > 0 and h > 0 and x+w <= bx+bw and y+h <= by+bh,
                "Invalid arm display ROI")
    stack_path = root / "stack-sleeve/atlas.json"
    if stack_path.exists():
        stack = manifest(stack_path)
        check_quality(stack, args)
        require(stack.get("response_kind") == "stack" and stack["source_sha256"] == source_hash
                and stack["base_image_sha256"] == beauty_hash, "Stack lighting belongs to another scene")
        require([stack["width"], stack["height"]] == dimensions, "Wrong stack scene resolution")
        for key in ("view_transform", "look", "exposure", "gamma"):
            require(stack[key] == beauty[key], "Stack colour settings differ")
        for key, size in [("illumination", stack["crop"][2]*stack["crop"][3]*8),
                          ("residual", stack["crop"][2]*stack["crop"][3]*8),
                          ("view_lut", stack["lut_edge"]**3*4)]:
            entry = stack[key]
            path = local_file(stack_path.parent, entry["file"])
            require(path.stat().st_size == size and digest(path) == entry["sha256"], "Corrupt stack " + key)
        print(f"PASS HQ previous-track stack sleeve: {stack_path}")
    check_png(root / "beauty.png", dimensions, 16)
    for label, data in [("poses", poses), ("guide", guide), ("sleeve", sleeve)]:
        require(data["source_sha256"] == source_hash, f"Mixed source scene in {label}")
    for data in (beauty, poses, sleeve):
        check_quality(data, args)
    require(poses["base_image_sha256"] == sleeve["base_image_sha256"] == beauty_hash,
            "Pose/sleeve packages belong to another beauty")
    require(sleeve["base_source_sha256"] == source_hash, "Sleeve uses an older base source")
    require(sleeve.get("region_compatibility") is None, "Expected a freshly matched sleeve, not legacy reuse")
    for key in ("view_transform", "look", "exposure", "gamma"):
        require(beauty[key] == sleeve[key], f"Sleeve colour settings differ: {key}")
    pose_hash = digest(root / "arm-poses/atlas.json")
    guide_hash = digest(root / "arm-guide.json")
    require(guide["pose_manifest_sha256"] == flow["pose_manifest_sha256"] == pose_hash,
            "Stale motion guide/flow package")
    require(flow["tube_guide_sha256"] == guide_hash, "Flow uses another geometry guide")
    require(flow["crop_width"] == poses["crop"][2] and flow["crop_height"] == poses["crop"][3]
            and 0 < flow["width"] <= flow["crop_width"]
            and 0 < flow["height"] <= flow["crop_height"], "Flow dimensions differ from pose crop")
    frames = poses["frames"]
    require(len(frames) == args.frames and len(flow["pairs"]) == len(frames)-1,
            "Incomplete pose/flow sequence")
    pose_images = []
    for index, frame in enumerate(frames):
        require(abs(frame["progress"]-index/(len(frames)-1)) < 1e-6, "Invalid pose progress")
        path = local_file(root / "arm-poses", frame["file"])
        check_png(path, poses["crop"][2:], 16)
        pose_images.append(digest(path))
    require(flow["pose_images_sha256"] == pose_images, "Flow was prepared from different images")
    for pair in flow["pairs"]:
        path = local_file(root / "arm-flow", pair["file"])
        require(digest(path) == pair["sha256"], "Corrupt flow field")
        check_png(path, (flow["width"], flow["height"]), 16)
    edge = flow["edge_layers"]
    require(edge["guide_sha256"] == guide_hash and len(edge["masks"]) == len(frames),
            "Incomplete/stale foreground coverage")
    for entry in [*edge["masks"], edge["background"]]:
        path = local_file(root / "arm-flow", entry["file"])
        require(digest(path) == entry["sha256"], "Corrupt foreground/background layer")
        check_png(path, poses["crop"][2:], 8)
    for key, size in [("illumination", sleeve["crop"][2]*sleeve["crop"][3]*8),
                      ("residual", sleeve["crop"][2]*sleeve["crop"][3]*8),
                      ("view_lut", sleeve["lut_edge"]**3*4)]:
        entry = sleeve[key]
        path = local_file(root / "sleeve", entry["file"])
        require(path.stat().st_size == size and digest(path) == entry["sha256"],
                f"Corrupt sleeve {key}")
    record_path = args.record_lighting or root / "record-lighting/atlas.json"
    if args.record_lighting is not None or record_path.exists():
        record = manifest(record_path)
        check_quality(record, args)
        require(record["response_kind"] == "record_lighting", "Record uses another paper surface")
        require(record["source_sha256"] == source_hash and record["base_image_sha256"] == beauty_hash,
                "Record lighting belongs to another scene/beauty")
        require([record["width"], record["height"]] == dimensions, "Wrong record scene resolution")
        require(record["excluded_moving_objects"] == poses["moving_objects"],
                "Record calibration must exclude the complete moving arm")
        for key in ("view_transform", "look", "exposure", "gamma"):
            require(record[key] == beauty[key], f"Record colour settings differ: {key}")
        x, y, w, h = record["crop"]
        require(min(x,y) >= 0 and min(w,h) > 0 and x+w <= dimensions[0] and y+h <= dimensions[1],
                "Invalid record lighting crop")
        for key, size in [("illumination",w*h*8), ("residual",w*h*8),
                          ("view_lut",record["lut_edge"]**3*4)]:
            entry = record[key]
            path = local_file(record_path.parent,entry["file"])
            require(path.stat().st_size == size and digest(path) == entry["sha256"],
                    f"Corrupt record {key}")
        work = record_path.parent / "work"
        if work.exists():
            for name in ("white.png","black.png"):
                check_png(work/name,(w,h),16)
        print(f"PASS HQ record lighting: {record_path}; {w}x{h}, stationary response, complete arm excluded")
    vinyl_path = args.vinyl_detail or root/'vinyl-detail/atlas.json'
    if args.vinyl_detail is not None or vinyl_path.exists():
        vinyl = manifest(vinyl_path)
        check_quality(vinyl, args)
        require(vinyl['response_kind']=='vinyl_detail', 'Wrong vinyl response kind')
        require(vinyl['source_sha256']==source_hash and vinyl['base_image_sha256']==beauty_hash,
                'Vinyl detail belongs to another source/beauty')
        require([vinyl['width'],vinyl['height']]==dimensions, 'Wrong vinyl resolution')
        require(vinyl['excluded_moving_objects']==poses['moving_objects'], 'Vinyl calibration includes moving arm')
        require(isinstance(vinyl['baked_detail'],bool), 'Missing original detail visibility')
        compatibility=vinyl.get('overlay_compatibility')
        if compatibility:
            require(compatibility['scope']=='vinyl-detail-texture-and-opacity-only' and vinyl['baked_detail'] is False,
                    'Invalid texture-only refresh scope')
            require(compatibility['base_source_sha256']==source_hash and
                    compatibility['updated_source_sha256']==vinyl['overlay_source_sha256'],
                    'Incorrect texture refresh source provenance')
            for key in ('unchanged_scene_sha256','unchanged_detail_response_sha256','base_response_sha256'):
                value=compatibility[key]
                require(len(value)==64 and all(c in '0123456789abcdef' for c in value), 'Invalid refresh signature')
            if 'source_scene_matches' in compatibility:
                require(type(compatibility['source_scene_matches']) is bool and
                        type(compatibility.get('other_source_changes_excluded')) is bool and
                        compatibility['source_scene_matches'] != compatibility['other_source_changes_excluded'],
                        'Ambiguous overlay-only refresh scope')
        if args.overlay_source:
            require(digest(args.overlay_source)==vinyl.get('overlay_source_sha256',vinyl['source_sha256']),
                    'Overlay source snapshot differs')
        for key in ('view_transform','look','exposure','gamma'):
            require(vinyl[key]==beauty[key], f'Wrong vinyl view setting {key}')
        x,y,w,h=vinyl['crop']
        require(min(x,y)>=0 and min(w,h)>0 and x+w<=dimensions[0] and y+h<=dimensions[1], 'Invalid vinyl crop')
        size=vinyl['detail_size']
        require(64<=size<=2048, 'Oversized vinyl texture')
        for key,length in [('illumination',w*h*8),('residual',w*h*8),
                           ('view_lut',vinyl['lut_edge']**3*4),('detail',size*size*4)]:
            entry=vinyl[key]
            path=local_file(vinyl_path.parent,entry['file'])
            require(path.stat().st_size==length and digest(path)==entry['sha256'], f'Corrupt vinyl {key}')
        work=vinyl_path.parent/'work'
        if work.exists():
            for name in ('clean.png','white.png','reference-zero.png','reference-rotated.png'):
                if (work/name).exists(): check_png(work/name,(w,h),16)
        print(f'PASS HQ vinyl detail: {vinyl_path}; {size}px premultiplied marks, fixed {w}x{h} response')
    button_path = root / 'buttons/atlas.json'
    if button_path.exists():
        buttons = manifest(button_path)
        require(buttons['format_version'] == 1
                and buttons['source_sha256'] == source_hash
                and buttons['base_image_sha256'] == beauty_hash
                and [buttons['width'], buttons['height']] == dimensions,
                'Buttons belong to another source/beauty')
        require(buttons['order'] == ['play_pause', 'previous', 'next']
                and 4 <= buttons['steps'] <= 16, 'Invalid button sequence')
        x, y, w, h = buttons['crop']
        require(min(x, y) >= 0 and 0 < w <= 512 and 0 < h <= 256
                and x+w <= dimensions[0] and y+h <= dimensions[1], 'Invalid button crop')
        check_quality({'samples': buttons['quality']['samples'], 'quality': buttons['quality']}, args)
        entry = buttons['poses']
        path = local_file(button_path.parent, entry['file'])
        expected = w*h*4*(1+3*buttons['steps'])
        require(path.stat().st_size == expected and digest(path) == entry['sha256'],
                'Corrupt button depth array')
        require(all(a == 255 for a in path.read_bytes()[3::4]), 'Button atlas is not opaque RGBA8')
        print(f'PASS HQ buttons: {button_path}; {w}x{h}, {1+3*buttons["steps"]} real depth layers')
    print(f"PASS HQ package: {root}; {dimensions}, {len(frames)} poses, "
          f"{args.samples}/{args.noise_threshold}, 16-bit masters, matched lighting/flow/sleeve; "
          f"source={source_hash}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenes", type=Path, required=True)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--record-lighting", type=Path,
                        help="Optional staged label calibration; otherwise verify the published one if present")
    parser.add_argument("--samples", type=int, default=2048)
    parser.add_argument('--vinyl-detail', type=Path, help='Optional staged rotating-surface calibration')
    parser.add_argument('--overlay-source',type=Path,help='Check the updated source for a verified texture-only refresh')
    parser.add_argument("--noise-threshold", type=float, default=.006)
    parser.add_argument("--min-samples", type=int, default=64)
    parser.add_argument("--frames", type=int, default=33)
    verify(parser.parse_args())


if __name__ == "__main__":
    main()
