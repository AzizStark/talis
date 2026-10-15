"""Offline quality-profile regressions; no Blender process or GPU required."""
import argparse
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch


class QualityTests(unittest.TestCase):
    def setUp(self):
        self.cycles = SimpleNamespace(with_openimagedenoise=True)
        self.mock = patch.dict(sys.modules, {"bpy": SimpleNamespace(), "_cycles": self.cycles})
        self.mock.start()
        sys.modules.pop("render_quality", None)
        import render_quality
        self.quality = render_quality
        parser = argparse.ArgumentParser()
        parser.add_argument("--samples", type=int, default=2048)
        render_quality.add_arguments(parser)
        self.args = parser.parse_args([])
        self.scene = SimpleNamespace(cycles=SimpleNamespace(),
            view_layers=[SimpleNamespace(name="ViewLayer", use=True,
                                         cycles=SimpleNamespace(use_denoising=False))],
            render=SimpleNamespace(image_settings=SimpleNamespace()))

    def tearDown(self):
        sys.modules.pop("render_quality", None)
        self.mock.stop()

    def test_production_defaults_are_shared_and_deterministic(self):
        self.quality.configure_quality(self.scene, self.args)
        c = self.scene.cycles
        self.assertEqual(c.samples, 2048)
        self.assertEqual(c.adaptive_threshold, .006)
        self.assertEqual(c.adaptive_min_samples, 64)
        self.assertTrue(c.use_adaptive_sampling)
        self.assertTrue(c.use_denoising)
        self.assertEqual(c.denoiser, "OPENIMAGEDENOISE")
        self.assertEqual(c.denoising_input_passes, "RGB_ALBEDO_NORMAL")
        self.assertEqual(c.denoising_prefilter, "ACCURATE")
        self.assertEqual(c.seed, 0)
        self.assertFalse(c.use_animated_seed)
        self.assertTrue(self.scene.view_layers[0].cycles.use_denoising)
        self.assertEqual(self.scene.render.image_settings.color_depth, "16")

    def test_invalid_quality_fails_before_mutating_scene(self):
        for key, value in [("samples", 0), ("noise_threshold", float("nan")),
                           ("noise_threshold", 0), ("noise_threshold", 2),
                           ("min_samples", -1), ("min_samples", 2049)]:
            with self.subTest(key=key, value=value):
                args = SimpleNamespace(**vars(self.args))
                setattr(args, key, value)
                with self.assertRaises(ValueError):
                    self.quality.configure_quality(self.scene, args)
                self.assertEqual(vars(self.scene.cycles), {})

    def test_missing_denoiser_is_not_a_silent_quality_downgrade(self):
        self.cycles.with_openimagedenoise = False
        with self.assertRaisesRegex(RuntimeError, "OpenImageDenoise"):
            self.quality.configure_quality(self.scene, self.args)
        self.assertEqual(vars(self.scene.cycles), {})

    def test_package_verifier_accepts_blender_float_precision_but_rejects_lower_quality(self):
        from verify_scene_bake import check_quality
        self.quality.configure_quality(self.scene, self.args)
        data = {"samples": self.args.samples,
                "quality": {**vars(self.scene.cycles), "png_color_depth": "16"}}
        data["quality"]["adaptive_threshold"] = .006000000052154064
        check_quality(data, self.args)
        for key, value in [("samples", 128), ("adaptive_threshold", .007),
                           ("png_color_depth", "8"), ("use_denoising", False)]:
            with self.subTest(key=key):
                altered = {**data, "quality": {**data["quality"], key: value}}
                with self.assertRaises(ValueError):
                    check_quality(altered, self.args)


if __name__ == "__main__":
    unittest.main()
