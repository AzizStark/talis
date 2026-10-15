"""Publish a hash-bound display ROI without changing poses, flow fields or UVs.

The right cut is an explicit artistic decision: faint indirect-light variation
there is frozen. Validate ALL foreground masks and a motion/filtering margin.
Run again after any pose/flow rebake; a stale region is ignored by the app.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

import cv2
import numpy as np


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('poses', type=Path)
    parser.add_argument('flow', type=Path)
    parser.add_argument('--right', type=int, required=True, help='Right edge in original crop-local pixels')
    args = parser.parse_args()
    poses, flow = json.loads(args.poses.read_text()), json.loads(args.flow.read_text())
    ph, fh = digest(args.poses), digest(args.flow)
    x, y, w, h = poses['crop']
    if not 32 < args.right <= w or flow['pose_manifest_sha256'] != ph:
        raise ValueError('Invalid cut or mismatched flow manifest')
    if (flow['crop_width'], flow['crop_height']) != (w, h):
        raise ValueError('Flow grid belongs to a different crop')
    motion_max = 0
    for pair in flow['pairs']:
        path = args.flow.parent / pair['file']
        if digest(path) != pair['sha256']:
            raise ValueError('Flow image changed')
        field = cv2.imread(str(path), cv2.IMREAD_UNCHANGED)
        motion_max = max(motion_max, float(np.abs((field.astype(np.float32)-32768)/256).max()))
    margin = 16 + math.ceil(motion_max) + 2
    max_foreground_x = 0
    for pose, expected, mask in zip(poses['frames'], flow['pose_images_sha256'], flow['edge_layers']['masks'], strict=True):
        if digest(args.poses.parent / pose['file']) != expected:
            raise ValueError('Pose image changed')
        path = args.flow.parent / mask['file']
        if digest(path) != mask['sha256']:
            raise ValueError('Foreground mask changed')
        image = cv2.imread(str(path), cv2.IMREAD_GRAYSCALE)
        if image.shape != (h, w):
            raise ValueError('Wrong foreground mask size')
        _, xx = np.nonzero(image)
        max_foreground_x = max(max_foreground_x, int(xx.max()) if len(xx) else 0)
    if max_foreground_x + margin >= args.right:
        raise ValueError('Cut intersects foreground motion/filtering support')
    if digest(args.poses) != ph or digest(args.flow) != fh:
        raise ValueError('Source manifests changed during validation')
    result = {'format_version': 1, 'pose_manifest_sha256': ph, 'flow_manifest_sha256': fh,
              'crop': [x, y, args.right, h]}
    output = args.poses.with_name('update-region.json')
    output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'output': str(output), 'crop': result['crop'], 'foreground_right': max_foreground_x,
                      'validated_margin_pixels': margin, 'arm_area_reduction_percent': 100*(1-args.right/w)}))


if __name__ == '__main__':
    main()
