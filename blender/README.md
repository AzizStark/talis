# Scene baking

Requires Python 3.11+ and Blender 4.0.2 with Cycles and OpenImageDenoise.
Edit `day.blend` / `night.blend`, pack textures, and preserve the named rig,
fixed camera and released-button frame 1.

Run from the repository root:

```sh
python3 -m venv blender/.venv
source blender/.venv/bin/activate
pip install -r blender/requirements.txt
python blender/bake.py --check
python blender/bake.py --output /path/to/bake
```

Defaults to both scenes, all tiers (1080p, 1440p, 4K), and automatic device
selection: OptiX → CUDA → CPU. Options include `--scene day`,
`--resolution 1440p`, `--device OPTIX` and `--blender /path/to/blender`.
See `--help` for all options.

## Resume / preview

```sh
python blender/bake.py --output /path/to/bake --resume
bash scripts/run.sh --config /path/to/bake/preview-1440p.toml
```

Resume reuses captured sources; use a fresh output directory for new edits.
Bakes need several GiB and never replace live `assets/` automatically.
Outputs retain 16-bit masters; shipped 8-bit/PNG optimization is separate.
Review crop coverage and hit regions after moving scene elements.

## Tests

```sh
PYTHONPATH=blender:blender/tools python -m unittest discover -s tests/blender
```

Asset licenses: [`LICENSING.md`](../LICENSING.md).
