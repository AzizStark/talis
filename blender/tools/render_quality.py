"""Shared offline Cycles quality profile; never save scene/preferences changes."""
import math
import _cycles


def add_arguments(parser):
    parser.add_argument("--noise-threshold", type=float, default=0.006)
    parser.add_argument("--min-samples", type=int, default=64)
    parser.add_argument("--color-depth", choices=("8", "16"), default="16")


def configure_quality(scene, args):
    if (args.samples < 1 or not math.isfinite(args.noise_threshold)
            or not 0 < args.noise_threshold <= 1
            or not 0 <= args.min_samples <= args.samples):
        raise ValueError("Positive samples/threshold and 0 <= minimum <= maximum samples required")
    if not _cycles.with_openimagedenoise:
        raise RuntimeError("The high-quality bake requires an OpenImageDenoise-enabled Blender build")
    scene.cycles.samples = args.samples
    scene.cycles.use_adaptive_sampling = True
    scene.cycles.adaptive_threshold = args.noise_threshold
    scene.cycles.adaptive_min_samples = args.min_samples
    scene.cycles.use_denoising = True
    scene.cycles.denoiser = "OPENIMAGEDENOISE"
    scene.cycles.denoising_input_passes = "RGB_ALBEDO_NORMAL"
    scene.cycles.denoising_prefilter = "ACCURATE"
    scene.cycles.seed = 0
    scene.cycles.use_animated_seed = False
    # A saved per-layer override can otherwise silently suppress scene-level
    # denoising. This is temporary bake state, never saved to the authoring file.
    for layer in scene.view_layers:
        if layer.use:
            layer.cycles.use_denoising = True
    scene.render.image_settings.color_depth = args.color_depth


def quality_metadata(scene):
    keys = ["samples", "use_adaptive_sampling", "adaptive_threshold", "adaptive_min_samples",
            "use_denoising", "denoiser", "denoising_input_passes", "denoising_prefilter",
            "seed", "use_animated_seed", "max_bounces", "diffuse_bounces", "glossy_bounces",
            "transmission_bounces", "transparent_max_bounces"]
    return {**{key: getattr(scene.cycles, key) for key in keys},
            "png_color_depth": scene.render.image_settings.color_depth,
            "view_layer_denoising": {layer.name: layer.cycles.use_denoising
                                     for layer in scene.view_layers if layer.use}}
