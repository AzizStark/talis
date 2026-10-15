"""Losslessly trim the right-hand sampling margin of a matched arm package.

Keep the wide Cycles/flow masters: changing a render border can change denoising,
and estimating flow again is not equivalent to cropping its existing grid.
Only an already verified, full-height, left-aligned display ROI is supported.
No animation bounds are inferred from equal endpoint colours.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import tempfile

import cv2
import numpy as np


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value, compact=False):
    path.write_text((json.dumps(value, separators=(",", ":")) if compact
                     else json.dumps(value, indent=2)) + "\n")


def vector_max(field):
    # Match the runtime's conservative, per-pair cadence metric, including both
    # directions and all pixels (not only projected foreground geometry).
    # OpenCV exposes PNG channels as BGRA; the shader/runtime read RGBA with
    # forward.xy in RG and backward.xy in BA. Pair the actual vector components.
    values = (field[..., [2, 1, 0, 3]].astype(np.float32) - 32768) / 256
    return float(np.sqrt(np.maximum(
        values[..., 0]**2 + values[..., 1]**2,
        values[..., 2]**2 + values[..., 3]**2).max()))


def packed_width(width, display_width, scale, fields):
    maximum = max(vector_max(field) for field in fields)
    # Three inverse iterations each replace p +/- t*D, rather than accumulating
    # offsets. The forward/backward consistency probes reach up to 2*D.
    # Four extra source pixels cover bilinear pose/half-size-flow footprints;
    # retain at least the shader's 16-pixel feather outside the display ROI.
    margin = max(16, math.ceil(2 * maximum) + 4)
    alignment = math.lcm(32, scale)
    keep = min(width, math.ceil((display_width + margin) / alignment) * alignment)
    # Cropping must not change the arm's independent update cadence. If a
    # discarded flow extremum matters, extend the package, even to full width.
    while keep < width and any(vector_max(a[:, :keep//scale]) != vector_max(a)
                               for a in fields):
        keep = min(width, keep + alignment)
    return keep, margin, maximum


def pack(scene, output):
    scene, output = scene.resolve(), output.resolve()
    require(output != scene and not output.is_relative_to(scene)
            and not scene.is_relative_to(output), "Packing output must be separate from its source")
    require(not output.exists(), "Use a fresh packing output; existing data is never overwritten")
    inputs = {}

    def read_file(path, expected=None):
        encoded = path.read_bytes()
        actual = hashlib.sha256(encoded).hexdigest()
        require(expected is None or actual == expected, f"Stale/corrupt packing input: {path}")
        inputs[path] = actual
        return encoded

    def read_json(relative):
        return json.loads(read_file(scene / relative))

    def image(folder, entry, dimensions, dtype, channels):
        name = entry["file"]
        require(Path(name).name == name and name not in ("", ".", ".."), "Nonlocal image filename")
        encoded = read_file(scene / folder / name, entry.get("sha256"))
        value = cv2.imdecode(np.frombuffer(encoded, np.uint8), cv2.IMREAD_UNCHANGED)
        require(value is not None and value.dtype == dtype
                and value.shape[:2] == tuple(reversed(dimensions))
                and (1 if value.ndim == 2 else value.shape[2]) == channels,
                f"Unexpected image format/dimensions: {name}")
        return value

    poses = read_json("arm-poses/atlas.json")
    flow = read_json("arm-flow/atlas.json")
    guide = read_json("arm-guide.json")
    region = read_json("arm-poses/update-region.json")
    pose_hash = inputs[scene / "arm-poses/atlas.json"]
    flow_hash = inputs[scene / "arm-flow/atlas.json"]
    guide_hash = inputs[scene / "arm-guide.json"]
    x, y, width, height = poses["crop"]
    require(poses["format_version"] == flow["format_version"] == guide["format_version"]
            == region["format_version"] == 1, "Unsupported arm package version")
    require(flow["encoding"] == "rgba16_unorm_bias32768_scale256", "Unsupported flow encoding")
    require(flow["pose_manifest_sha256"] == guide["pose_manifest_sha256"]
            == region["pose_manifest_sha256"] == pose_hash
            and region["flow_manifest_sha256"] == flow_hash
            and flow["tube_guide_sha256"] == flow["edge_layers"]["guide_sha256"] == guide_hash,
            "Stale guide, flow, or display boundary")
    require(guide["source_sha256"] == poses["source_sha256"], "Mixed source scenes")
    require(poses["base_image_sha256"] == hashlib.sha256(read_file(scene / "beauty.png")).hexdigest(),
            "Arm package belongs to another beauty")
    require(region["crop"][:2] == [x, y] and region["crop"][3] == height
            and 0 < region["crop"][2] <= width, "Only left-aligned, full-height display regions can be packed")
    require(flow["crop_width"] == width and flow["crop_height"] == height
            and 0 < flow["width"] <= width and 0 < flow["height"] <= height
            and width % flow["width"] == height % flow["height"] == 0
            and width//flow["width"] == height//flow["height"],
            "Flow grid must have an exact, uniform integer scale; resampling is forbidden")
    scale = width // flow["width"]
    require(2 <= len(poses["frames"]) <= 128 and len(flow["pairs"]) == len(poses["frames"])-1
            and len(flow["pose_images_sha256"]) == len(poses["frames"])
            and len(flow["edge_layers"]["masks"]) == len(poses["frames"]), "Incomplete arm sequence")
    fields = [image("arm-flow", entry, (flow["width"], flow["height"]), np.uint16, 4)
              for entry in flow["pairs"]]
    keep, margin, maximum = packed_width(width, region["crop"][2], scale, fields)
    require(keep % scale == 0, "Packed flow texel alignment differs")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=".arm-pack-", dir=output.parent))
    (temporary / "arm-poses").mkdir()
    (temporary / "arm-flow").mkdir()

    def save(folder, entry, data, columns):
        target = temporary / folder / entry["file"]
        # PNG is lossless; explicitly round-trip the stored values, including
        # 16-bit masters/flow and premultiplied background alpha.
        subset = np.ascontiguousarray(data[:, :columns])
        require(cv2.imwrite(str(target), subset), f"Cannot encode {target}")
        require(np.array_equal(cv2.imread(str(target), cv2.IMREAD_UNCHANGED), subset),
                f"PNG round-trip changed pixels: {target}")
        return {**entry, "sha256": digest(target)}

    packed_poses = copy.deepcopy(poses)
    packed_poses["crop"][2] = keep
    packed_poses["crop_method"] = "Lossless runtime packing of verified wide Cycles masters"
    hashes = []
    for frame, expected in zip(poses["frames"], flow["pose_images_sha256"]):
        entry = {"file": frame["file"], "sha256": expected}
        data = image("arm-poses", entry, (width, height), np.uint16, 3)
        hashes.append(save("arm-poses", entry, data, keep)["sha256"])
    write_json(temporary / "arm-poses/atlas.json", packed_poses)
    new_pose_hash = digest(temporary / "arm-poses/atlas.json")
    packed_guide = copy.deepcopy(guide)
    # Left/top are unchanged: the original camera projection stays exact.
    packed_guide["pose_manifest_sha256"] = new_pose_hash
    # Geometry arrays use the exporter's compact form; only the pose binding
    # changes, not hundreds of thousands of formatting-only repository lines.
    write_json(temporary / "arm-guide.json", packed_guide, compact=True)
    new_guide_hash = digest(temporary / "arm-guide.json")
    packed_flow = copy.deepcopy(flow)
    packed_flow.update(width=keep//scale, crop_width=keep,
                       pose_manifest_sha256=new_pose_hash, pose_images_sha256=hashes,
                       tube_guide_sha256=new_guide_hash)
    packed_flow["pairs"] = [save("arm-flow", entry, data, keep//scale)
                            for entry, data in zip(flow["pairs"], fields)]
    edge = packed_flow["edge_layers"]
    edge["guide_sha256"] = new_guide_hash
    edge["masks"] = [save("arm-flow", entry,
                          image("arm-flow", entry, (width, height), np.uint8, 1), keep)
                     for entry in flow["edge_layers"]["masks"]]
    entry = flow["edge_layers"]["background"]
    edge["background"] = save("arm-flow", entry,
                              image("arm-flow", entry, (width, height), np.uint8, 4), keep)
    write_json(temporary / "arm-flow/atlas.json", packed_flow)
    packed_region = {**region, "pose_manifest_sha256": new_pose_hash,
                     "flow_manifest_sha256": digest(temporary / "arm-flow/atlas.json")}
    write_json(temporary / "arm-poses/update-region.json", packed_region)
    report = {"format_version": 1, "master_crop": poses["crop"], "packed_crop": packed_poses["crop"],
              "display_crop": region["crop"], "sampling_margin": margin,
              "maximum_flow_pixels": maximum, "flow_scale": scale,
              "master_inputs": {str(p.relative_to(scene)): h for p, h in inputs.items()},
              "cadence_bounds_preserved": True,
              "gpu_texture_bytes_saved": (width-keep)*height*(len(poses["frames"])*4+4)
                  + (width-keep)//scale*(height//scale)*len(fields)*8}
    write_json(temporary / "arm-poses/packing.json", report)
    require(all(digest(path) == expected for path, expected in inputs.items()),
            "A packing source changed; output has not been published")
    temporary.rename(output)
    return report


def install_staged(scene, packed):
    """Replace only private staged arm assets; rollback and preserve wide masters."""
    scene, packed = scene.resolve(), packed.resolve()
    repo = Path(__file__).resolve().parents[2]
    require(not scene.is_relative_to((repo / "assets").resolve()),
            "Live assets require the complete-pair publisher, not staged installation")
    require(not any(p.startswith(("scene-refresh-backup.", "arm-masters-")) for p in scene.parts),
            "Saved master backups are read-only; pack into a separate stage")
    backup = Path(tempfile.mkdtemp(prefix="arm-masters-", dir=packed.parent))
    names = ("arm-poses", "arm-flow", "arm-guide.json")
    moved = []
    try:
        for name in names:
            (scene / name).rename(backup / name)
            moved.append(name)
            (packed / name).rename(scene / name)
    except BaseException:
        for name in reversed(moved):
            if (scene / name).exists():
                (scene / name).rename(packed / name)
            (backup / name).rename(scene / name)
        raise
    return backup


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scene", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="Fresh, private arm-only output directory")
    parser.add_argument("--install-staged", action="store_true",
                        help="Replace arm files in a private bake stage, retaining wide masters beside the output")
    args = parser.parse_args()
    report = pack(args.scene, args.output)
    if args.install_staged:
        report["masters_backup"] = str(install_staged(args.scene, args.output))
    print("TALIS_ARM_PACKED " + json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
