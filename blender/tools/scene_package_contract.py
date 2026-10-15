"""Offline scene-layer contract shared by preparation and publication tools.

Optional means a legacy package may omit the feature, not that a refresh may
silently remove it. Geometry/material changes require matched calibration;
non-overlapping screen rectangles do not prove indirect lighting is unchanged.
"""
from dataclasses import dataclass
import json


@dataclass(frozen=True)
class Layer:
    folder: str
    stage: str
    required: bool = True
    refresh_instruction: str = ""

    @property
    def manifest(self):
        return self.folder + "/atlas.json"


LAYERS = (
    Layer("arm-poses", "arm-poses"),
    Layer("sleeve", "sleeve"),
    Layer("stack-sleeve", "stack-sleeve", False, "bake the stack-sleeve stage"),
    Layer("empty-stand", "empty-stand", False, "bake the empty-stand stage"),
    Layer("record-lighting", "record"),
    Layer("vinyl-detail", "vinyl"),
    Layer("buttons", "button-package"),
    Layer("knob", "knob", False, "bake with --with-knob"),
)
REQUIRED_ASSETS = ("beauty.png", "beauty.json", "arm-guide.json", "arm-flow/atlas.json",
                   *(layer.manifest for layer in LAYERS if layer.required))
BAKE_STAGES = ("beauty", "arm-poses", "arm-guide", "flow", "edges", "arm-region",
               "sleeve", "stack-sleeve", "empty-stand", "record", "vinyl", "buttons",
               "button-package", "knob", "arm-pack", "verify")
CENTER_SCOPE = "center-spindle-and-default-label-only"
CENTER_UPDATE_UNSUPPORTED = (
    "Scoped centre refresh is retired: the old crop bake does not refresh or prove "
    "compatibility of lighting, dust, knob, stack and cover-OFF dependencies. "
    "No assets have been changed. Use python blender/bake.py to stage a "
    "full matched bake; verify both modes before publishing."
)


def package_root(pair, variant, *, allow_legacy=False):
    """New pairs are symmetric; old private stages/backups remain readable."""
    if variant not in ("day", "night"):
        raise ValueError("Unknown scene variant")
    flat = (pair / "beauty.json").exists() or (pair / "beauty.png").exists()
    if (pair / "day").exists():
        if flat:
            raise ValueError("Mixed scene layout: day/ and flat day assets coexist")
        return pair / variant
    if allow_legacy and flat:
        return pair if variant == "day" else pair / "night"
    return pair / variant


def require_complete_scene(root, variant="scene"):
    for name in REQUIRED_ASSETS:
        if not (root / name).is_file():
            raise ValueError(f"Incomplete {variant} scene package: {name}")


def validate_base_bindings(root, beauty_hash):
    """Check every known present layer, including optional legacy capabilities."""
    for layer in LAYERS:
        path = root / layer.manifest
        if path.exists():
            data = json.loads(path.read_text())
            if data.get("base_image_sha256") != beauty_hash:
                raise ValueError(f"Stale {layer.folder}: base image hash differs")


def reject_legacy_center_refresh(beauty):
    # Hash rebinding is not evidence that an old calibration is still valid.
    # The historical 2026-10-05 package is superseded by matched full bakes.
    if beauty.get("regional_refresh", {}).get("scope") == CENTER_SCOPE:
        raise ValueError(CENTER_UPDATE_UNSUPPORTED)


def reject_center_update(root):
    """Read-only diagnostic for old commands; never merge or rebind any layer."""
    present = [layer.folder for layer in LAYERS if (root / layer.manifest).is_file()]
    missing = [name for name in REQUIRED_ASSETS if not (root / name).is_file()]
    detail = " Present layers: " + ", ".join(present) if present else ""
    if missing:
        detail += " Missing assets: " + ", ".join(missing)
    raise ValueError(CENTER_UPDATE_UNSUPPORTED + detail)
