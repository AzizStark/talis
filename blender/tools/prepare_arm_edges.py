"""Prepare foreground coverage and a background reference for arm/disk overlaps."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import warnings

import cv2
import numpy as np

from prepare_arm_flow import digest, tube_guide, read_rgb8


def save_image(path, image):
    with tempfile.NamedTemporaryFile(dir=path.parent, suffix=".png", delete=False) as temp:
        temporary = Path(temp.name)
    try:
        if not cv2.imwrite(str(temporary), image):
            raise RuntimeError(f"Could not save {path.name}")
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def prepare(poses_path, guide_path, atlas_path):
    poses = json.loads(poses_path.read_text())
    guide = json.loads(guide_path.read_text())
    atlas = json.loads(atlas_path.read_text())
    pose_hash = digest(poses_path)
    guide_hash = digest(guide_path)
    if atlas["pose_manifest_sha256"] != pose_hash or guide["pose_manifest_sha256"] != pose_hash:
        raise ValueError("Edge assets must match the pose/flow package")
    width, height = poses["crop"][2:]
    _, scopes = tube_guide(guide_path, poses, pose_hash, (atlas["width"], atlas["height"]))
    scope = cv2.resize(np.maximum.reduce(scopes), (width, height))
    if "foreground_vertices" not in guide:
        raise ValueError("Re-export the guide with export_arm_guide.py to include foreground geometry")
    vertices = np.asarray(guide["foreground_vertices"], np.float64)
    vertices = np.column_stack((vertices, np.ones(len(vertices))))
    triangles = np.asarray(guide["foreground_triangles"], np.int32)
    projection = np.asarray(guide["projection"], np.float64)
    px, py, _ = poses["pivot_blender"]
    masks, images, entries = [], [], []
    cv2.setNumThreads(4)
    for index, frame in enumerate(poses["frames"]):
        angle = np.radians(frame["angle_degrees"])
        c, s = np.cos(angle), np.sin(angle)
        rotate = np.array([[c, -s, 0, px-c*px+s*py], [s, c, 0, py-s*px-c*py],
                           [0, 0, 1, 0], [0, 0, 0, 1]])
        projected = vertices @ (projection @ rotate).T
        if np.min(projected[:, 2]) <= 0:
            raise ValueError("Foreground geometry crosses the camera plane")
        # Four subpixels per axis, followed by a small pixel-filter approximation.
        points = np.rint((projected[:, :2]/projected[:, 2:]*4-.5)*16).astype(np.int32)
        high = np.zeros((height*4, width*4), np.uint8)
        for triangle in triangles:
            cv2.fillConvexPoly(high, points[triangle], 255, shift=4)
        mask = cv2.resize(high, (width, height), interpolation=cv2.INTER_AREA)
        mask = cv2.GaussianBlur(mask, (0, 0), .40)
        image_path = poses_path.parent/frame["file"]
        if digest(image_path) != atlas["pose_images_sha256"][index]:
            raise ValueError("Pose image differs from the flow package")
        image = read_rgb8(image_path)
        if image is None or image.shape != (height, width, 3):
            raise ValueError("Invalid pose image")
        masks.append(mask)
        images.append(image)
        name = f"foreground-{index:03}.png"
        destination = atlas_path.parent/name
        save_image(destination, mask)
        entries.append({"file": name, "sha256": digest(destination)})
        print(f"TALIS_EDGE_MASK {index+1}/{len(poses['frames'])}", flush=True)
    masks = np.stack(masks)
    images = np.stack(images)
    clear = masks < 2
    observed = clear.any(axis=0)
    background = np.zeros((height, width, 4), np.uint8)
    # Median of genuinely uncovered pixels; never invent a background at pixels
    # hidden in every source pose. Small row blocks keep preparation memory bounded.
    for row in range(0, height, 32):
        block = images[:, row:row+32].astype(np.float32)
        block[~clear[:, row:row+32]] = np.nan
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", RuntimeWarning)
            median = np.nanmedian(block, axis=0)
        background[row:row+32, :, :3] = np.rint(np.nan_to_num(median)).astype(np.uint8)
    # Exclude the transition next to unobserved background; no correction there.
    distance = cv2.distanceTransform(observed.astype(np.uint8), cv2.DIST_L2, cv2.DIST_MASK_PRECISE)
    validity = np.clip(distance/3, 0, 1)
    background[..., 3] = np.rint(scope*validity*255).astype(np.uint8)
    destination = atlas_path.parent/"tube-background.png"
    save_image(destination, background)
    if (digest(poses_path) != pose_hash or digest(guide_path) != guide_hash
            or [digest(poses_path.parent/frame["file"]) for frame in poses["frames"]] != atlas["pose_images_sha256"]):
        raise RuntimeError("Sources changed during edge preparation")
    atlas["edge_layers"] = {
        "masks": entries,
        "background": {"file": destination.name, "sha256": digest(destination)},
        "guide_sha256": guide_hash,
        "method": "Projected foreground coverage; median of uncovered pose pixels; scoped background correction",
    }
    # Publish this optional extension last, keeping old flow-only packages usable.
    temporary = atlas_path.with_suffix(".edges.tmp")
    temporary.write_text(json.dumps(atlas, indent=2)+"\n")
    temporary.replace(atlas_path)
    print(f"TALIS_EDGES_COMPLETE extra_texture_mib={width*height*4/1048576:.4f}; masks use existing pose alpha")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--poses", type=Path, default=Path("assets/scenes/day/arm-poses/atlas.json"))
    parser.add_argument("--guide", type=Path, default=Path("assets/scenes/day/arm-guide.json"))
    parser.add_argument("--atlas", type=Path, default=Path("assets/scenes/day/arm-flow/atlas.json"))
    args = parser.parse_args()
    prepare(args.poses, args.guide, args.atlas)
