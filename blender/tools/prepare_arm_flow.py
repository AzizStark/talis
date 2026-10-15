"""Cache bidirectional DIS motion fields; no intermediate beauty frames are baked."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

import cv2
import numpy as np

BIAS = 32768
SCALE = 256


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_rgb8(path):
    """BGR display bytes with the same u16 rounding as Rust image::to_rgba8.

    OpenCV's default colour load truncates a 16-bit PNG instead. Keep motion
    estimation and median edge backgrounds in the runtime texture's exact space.
    """
    image = cv2.imread(str(path), cv2.IMREAD_UNCHANGED)
    if image is None:
        return None
    if image.dtype == np.uint16:
        image = ((image.astype(np.uint32) + 128) // 257).astype(np.uint8)
    if image.dtype != np.uint8 or image.ndim != 3 or image.shape[2] not in (3, 4):
        raise ValueError("Expected an RGB/RGBA 8- or 16-bit pose PNG")
    return image[..., :3]


def estimate(a, b, grid):
    # Motion is estimated in encoded image space, matching the cached RGB assets.
    a = cv2.resize(a, grid, interpolation=cv2.INTER_AREA)
    b = cv2.resize(b, grid, interpolation=cv2.INTER_AREA)
    gray_a, gray_b = (cv2.cvtColor(im, cv2.COLOR_BGR2GRAY) for im in (a, b))
    solver = cv2.DISOpticalFlow_create(cv2.DISOPTICAL_FLOW_PRESET_MEDIUM)
    solver.setFinestScale(0)
    solver.setPatchStride(3)
    solver.setGradientDescentIterations(32)
    solver.setVariationalRefinementIterations(8)
    forward = solver.calc(gray_a, gray_b, None)
    backward = solver.calc(gray_b, gray_a, None)
    # Suppress spurious motion in the identical, stationary surroundings. Dilating
    # covers both silhouettes and their small displacements before the soft gate.
    delta = np.max(np.abs(a.astype(np.int16) - b.astype(np.int16)), axis=2)
    active = cv2.dilate((delta > 1).astype(np.float32), np.ones((9, 9), np.uint8))
    active = cv2.GaussianBlur(active, (0, 0), 2.0)
    return np.concatenate((forward * active[..., None], backward * active[..., None]), axis=2)


def encode(flow):
    if not np.isfinite(flow).all() or np.max(np.abs(flow)) >= 127:
        raise ValueError("Invalid or out-of-range motion; flow cache was not published")
    return np.rint(flow * SCALE + BIAS).astype(np.uint16)


def tube_guide(path, poses, source_hash, grid):
    guide = json.loads(path.read_text())
    if (guide["format_version"] != 1 or guide["pose_manifest_sha256"] != source_hash
            or guide["source_sha256"] != poses["source_sha256"]):
        raise ValueError("Stale tube guide; export it again from the original Blender scene")
    projection = np.asarray(guide["projection"], dtype=np.float64)
    vertices = np.asarray(guide["vertices"], dtype=np.float64)
    triangles = np.asarray(guide["triangles"], dtype=np.int32)
    if (projection.shape != (3, 4) or vertices.ndim != 2 or vertices.shape[1] != 3
            or triangles.ndim != 2 or triangles.shape[1] != 3 or not len(triangles)
            or np.min(triangles) < 0 or np.max(triangles) >= len(vertices)
            or not np.isfinite(projection).all() or not np.isfinite(vertices).all()
            or not np.isfinite(guide["plane_z"])):
        raise ValueError("Invalid tube projection/mesh")
    vertices = np.column_stack((vertices, np.ones(len(vertices))))
    plane = np.array([[1, 0, 0], [0, 1, 0], [0, 0, guide["plane_z"]], [0, 0, 1]])
    px, py, _ = poses["pivot_blender"]
    crop_size = np.array(poses["crop"][2:], dtype=np.float64)
    maps, masks = [], []
    for frame in poses["frames"]:
        angle = np.radians(frame["angle_degrees"])
        c, s = np.cos(angle), np.sin(angle)
        rotate = np.array([[c, -s, 0, px-c*px+s*py], [s, c, 0, py-s*px-c*py],
                           [0, 0, 1, 0], [0, 0, 0, 1]])
        maps.append(projection @ rotate @ plane)
        projected = vertices @ (projection @ rotate).T
        if np.min(projected[:, 2]) <= 0:
            raise ValueError("Tube guide crosses the camera plane")
        pixels = projected[:, :2] / projected[:, 2:]
        points = np.rint((pixels * np.array(grid) / crop_size - 0.5) * 16).astype(np.int32)
        mask = np.zeros((grid[1], grid[0]), dtype=np.uint8)
        for triangle in triangles:
            cv2.fillConvexPoly(mask, points[triangle], 1, shift=4)
        if not mask.any():
            raise ValueError("Tube guide is outside the crop")
        # Cover the tube and inverse-gather neighborhood with a coherent motion
        # field; only the outer margin blends back into image-estimated flow.
        # Distances use full-crop pixels, so --scale does not change the margin.
        distance = cv2.distanceTransform(1-mask, cv2.DIST_L2, cv2.DIST_MASK_PRECISE)
        distance *= float(np.mean(crop_size / np.array(grid)))
        t = np.clip((distance-6.0)/6.0, 0, 1)
        masks.append((1-t*t*(3-2*t)).astype(np.float32))
    return maps, masks


def plane_motion(source, target, grid, crop_size):
    y, x = np.mgrid[:grid[1], :grid[0]]
    pixels = np.stack(((x+0.5)*crop_size[0]/grid[0],
                       (y+0.5)*crop_size[1]/grid[1], np.ones_like(x)), axis=-1)
    projected = pixels @ (target @ np.linalg.inv(source)).T
    return (projected[..., :2]/projected[..., 2:] - pixels[..., :2]).astype(np.float32)


def guide_flow(flow, maps, masks, index, grid, crop_size):
    for direction, source, target in [(0, index, index+1), (2, index+1, index)]:
        rigid = plane_motion(maps[source], maps[target], grid, crop_size)
        weight = masks[source][..., None]
        flow[..., direction:direction+2] = (flow[..., direction:direction+2]*(1-weight)
                                             + rigid*weight)
    return flow


def self_test():
    with tempfile.TemporaryDirectory(prefix="talis-flow-depth-test-") as folder:
        values = np.arange(65536, dtype=np.uint16).reshape(256, 256)
        pixels = np.repeat(values[..., None], 3, axis=2)
        path = Path(folder)/"master.png"
        if not cv2.imwrite(str(path), pixels):
            raise RuntimeError("Could not create conversion fixture")
        expected = ((pixels.astype(np.uint32)+128)//257).astype(np.uint8)
        assert np.array_equal(read_rgb8(path), expected), "16-bit runtime colour rounding differs"
    rng = np.random.default_rng(7)
    a = cv2.GaussianBlur(rng.integers(0, 256, (128, 128, 3), dtype=np.uint8), (3, 3), 0)
    b = cv2.warpAffine(a, np.float32([[1, 0, 2], [0, 1, -1]]), (128, 128), borderMode=cv2.BORDER_REFLECT)
    flow = estimate(a, b, (128, 128))
    assert np.allclose(np.median(flow[20:-20, 20:-20, :2], axis=(0, 1)), [2, -1], atol=0.25)
    assert np.allclose(np.median(flow[20:-20, 20:-20, 2:], axis=(0, 1)), [-2, 1], atol=0.25)
    restored = (encode(flow).astype(np.float32) - BIAS) / SCALE
    assert np.max(np.abs(restored - flow)) <= 0.5 / SCALE + 0.00001
    assert np.max(np.abs(estimate(a, a, (128, 128)))) == 0
    identity = np.eye(3)
    translation = np.array([[1, 0, 2], [0, 1, -1], [0, 0, 1]])
    maps = [identity, translation]
    masks = [np.ones((8, 8), np.float32)] * 2
    guided = guide_flow(np.zeros((8, 8, 4), np.float32), maps, masks, 0, (8, 8), (16, 16))
    assert np.allclose(guided[..., :2], [2, -1])
    assert np.allclose(guided[..., 2:], [-2, 1])
    assert np.allclose(plane_motion(identity, identity, (8, 8), (16, 16)), 0)
    masks = [np.zeros((8, 8), np.float32)] * 2
    assert np.allclose(guide_flow(flow[:8, :8].copy(), maps, masks, 0, (8, 8), (16, 16)), flow[:8, :8])
    print("Flow preparation self-test passed: translation, direction, encoding, stationary images, tube guidance")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--poses", type=Path, default=Path("assets/scenes/day/arm-poses/atlas.json"))
    parser.add_argument("--output", type=Path, default=Path("assets/scenes/day/arm-flow"))
    parser.add_argument("--scale", type=float, default=0.5, help="Flow-grid scale; RGB poses remain full resolution")
    parser.add_argument("--guide", type=Path, default=Path("assets/scenes/day/arm-guide.json"))
    parser.add_argument("--no-guide", action="store_true", help="Explicitly use image-estimated flow only")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not 0.25 <= args.scale <= 1:
        parser.error("Use a flow-grid scale from 0.25 to 1")
    if args.output.resolve() == args.poses.resolve().parent:
        parser.error("Flow output must be separate from the source pose folder")
    source_hash = digest(args.poses)
    poses = json.loads(args.poses.read_text())
    if poses["format_version"] != 1 or not 2 <= len(poses["frames"]) <= 128:
        raise ValueError("Unsupported pose manifest")
    width, height = poses["crop"][2:]
    grid = (int(np.ceil(width * args.scale)), int(np.ceil(height * args.scale)))
    paths = []
    for frame in poses["frames"]:
        name = frame["file"]
        if Path(name).name != name or name in (".", ".."):
            raise ValueError("Unsafe pose filename")
        paths.append(args.poses.parent / name)
    hashes = [digest(path) for path in paths]
    guidance = None
    guide_hash = None
    if not args.no_guide:
        guidance = tube_guide(args.guide, poses, source_hash, grid)
        guide_hash = digest(args.guide)
    args.output.mkdir(parents=True, exist_ok=True)
    pairs = []
    cv2.setNumThreads(4)
    a = read_rgb8(paths[0])
    for index, path in enumerate(paths[1:]):
        b = read_rgb8(path)
        if a is None or b is None or a.shape != (height, width, 3) or b.shape != a.shape:
            raise ValueError("Pose image dimensions differ from the manifest")
        flow = estimate(a, b, grid)
        # Stored vectors are in FULL-CROP pixels, independent of the flow-grid size.
        flow *= np.float32([width / grid[0], height / grid[1]] * 2)
        if guidance is not None:
            flow = guide_flow(flow, *guidance, index, grid, (width, height))
        rgba = encode(flow)
        file = f"flow-{index:03}.png"
        with tempfile.NamedTemporaryFile(dir=args.output, suffix=".png", delete=False) as temp:
            temporary = Path(temp.name)
        try:
            # OpenCV writes BGRA; Rust/image decodes RGBA. RGBA is fwd.xy,bwd.xy.
            if not cv2.imwrite(str(temporary), rgba[..., [2, 1, 0, 3]]):
                raise RuntimeError("Failed to encode motion field")
            os.replace(temporary, args.output / file)
        finally:
            temporary.unlink(missing_ok=True)
        pairs.append({"file": file, "sha256": digest(args.output / file)})
        print(f"TALIS_FLOW_PAIR {index + 1}/{len(paths) - 1} max_pixels={np.max(np.abs(flow)):.3f}", flush=True)
        a = b
    if (digest(args.poses) != source_hash or [digest(path) for path in paths] != hashes
            or (guide_hash is not None and digest(args.guide) != guide_hash)):
        raise RuntimeError("Source poses changed while estimating flow; cache was not published")
    manifest = {
        "format_version": 1, "algorithm": "OpenCV DIS medium, bidirectional, static-region gated",
        "opencv_version": cv2.__version__, "width": grid[0], "height": grid[1],
        "crop_width": width, "crop_height": height, "encoding": "rgba16_unorm_bias32768_scale256",
        "pose_manifest_sha256": source_hash, "pose_images_sha256": hashes, "pairs": pairs,
        "limitations": ["Experimental image warping, not geometry interpolation",
                         "Thin edges, reflections and disocclusions may still distort or step"],
    }
    if guidance is not None:
        manifest["algorithm"] += "; projected rigid center-plane guide for steel tube"
        manifest["tube_guide_sha256"] = guide_hash
    with tempfile.NamedTemporaryFile(dir=args.output, suffix=".json", mode="w", delete=False) as temp:
        json.dump(manifest, temp, indent=2)
        temp.write("\n")
        temporary = Path(temp.name)
    os.replace(temporary, args.output / "atlas.json")
    if guidance is not None:
        from prepare_arm_edges import prepare
        prepare(args.poses, args.guide, args.output / "atlas.json")
    print(f"TALIS_FLOW_COMPLETE {grid[0]}x{grid[1]}, {len(pairs)} cached fields; original poses unchanged")


if __name__ == "__main__":
    main()
