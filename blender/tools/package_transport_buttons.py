"""Package a validated cropped button bake as one small GPU texture array."""
import argparse
import hashlib
import json
from pathlib import Path
from PIL import Image, ImageChops

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("bake", type=Path)
parser.add_argument("beauty", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
m = json.loads((args.bake / "bake.json").read_text())
beauty = Image.open(args.beauty)
if beauty.size != (m["width"], m["height"]):
    raise ValueError("Button and beauty dimensions differ")
_, _, width, height = m["crop"]
images = [Image.open(args.bake / file).convert("RGBA") for file in m.pop("renders")]
if len(images) != 1 + 3*m["steps"] or any(i.size != (width, height) for i in images):
    raise ValueError("Unexpected pose count or crop dimensions")
for index in range(3):
    if not ImageChops.difference(images[0].convert("RGB"), images[(index+1)*m["steps"]].convert("RGB")).getbbox():
        raise ValueError("Button driver did not move")
pixels = b"".join(image.tobytes() for image in images)
args.output.mkdir(parents=True, exist_ok=True)
(args.output / "poses.rgba").write_bytes(pixels)
m["poses"] = {"file": "poses.rgba", "sha256": hashlib.sha256(pixels).hexdigest()}
m["base_image_sha256"] = hashlib.sha256(args.beauty.read_bytes()).hexdigest()
(args.output / "atlas.json").write_text(json.dumps(m, indent=2) + "\n")
print(f"Packaged {len(images)} real poses, {width}x{height}, {len(pixels)/1048576:.2f} MiB")
