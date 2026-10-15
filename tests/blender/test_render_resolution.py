"""Offline resolution planning; no Blender or GPU needed."""
import unittest
from fractions import Fraction
from render_resolution import arm_crop, button_crop, pixel_grid, validate_resolution
from verify_scene_bake import valid_knob_crop


class ResolutionTests(unittest.TestCase):
    def test_button_crop_is_grid_exact_and_runtime_bounded(self):
        for size in ((1920,1080),(2560,1440),(3840,2160)):
            x,y,w,h = button_crop(*size, [[.35,.63,.42,.66]])
            self.assertTrue(0 < w <= 512 and 0 < h <= 256)
            if size == (2560,1440):
                self.assertTrue(all(value % 32 == 0 for value in (x,y,w,h)))
                continue  # Keep the already-rendered 1440p button grid unchanged.
            for value,dimension in ((x,size[0]),(x+w,size[0]),(y,size[1]),(y+h,size[1])):
                denominator = Fraction(value,dimension).denominator
                self.assertEqual(denominator & (denominator-1),0)
        for boxes in ([], [[0,0,1,1]], [[float('nan'),.1,.2,.3]], [[-.1,.1,.2,.3]]):
            with self.assertRaises(ValueError): button_crop(3840,2160,boxes)

    def test_padded_knob_is_valid_but_existing_pixel_limit_is_preserved(self):
        for crop in [(2370,1215,120,270),(0,0,256,256),(0,0,512,128)]:
            self.assertTrue(valid_knob_crop(crop,(3840,2160)))
        for crop in [(0,0,257,256),(0,0,512,129),(0,0,513,1),(0,0,1,513),
                     (0,0,0,270),(3800,1215,120,270),(2370,2000,120,270),(-1,0,120,270)]:
            self.assertFalse(valid_knob_crop(crop,(3840,2160)))

    def test_existing_1440p_borders_are_unchanged(self):
        self.assertEqual(pixel_grid(2560,1440),(20,45))
        self.assertEqual(arm_crop(2560,1440),(1160,585,960,630))

    def test_4k_border_is_integer_exact_and_outward(self):
        self.assertEqual(pixel_grid(3840,2160),(30,135))
        self.assertEqual(arm_crop(3840,2160),(1740,810,1440,1080))
        x,y,w,h=arm_crop(3840,2160)
        self.assertLessEqual(y,585*1.5)
        self.assertGreaterEqual(y+h,1215*1.5)
        for value,dimension in [(x,3840),(x+w,3840),(y,2160),(y+h,2160)]:
            denominator=Fraction(value,dimension).denominator
            self.assertEqual(denominator & (denominator-1),0)

    def test_1080p_border_is_integer_exact_and_outward(self):
        self.assertEqual(pixel_grid(1920,1080),(15,135))
        self.assertEqual(arm_crop(1920,1080),(870,405,720,540))
        x,y,w,h=arm_crop(1920,1080)
        self.assertLessEqual(y,585*0.75)
        self.assertGreaterEqual(y+h,1215*0.75)
        for value,dimension in [(x,1920),(x+w,1920),(y,1080),(y+h,1080)]:
            denominator=Fraction(value,dimension).denominator
            self.assertEqual(denominator & (denominator-1),0)

    def test_invalid_or_stale_explicit_crop_fails(self):
        for size in [(0,2160),(3840,0),(20000,2160)]:
            with self.assertRaises(ValueError): validate_resolution(*size)
        for crop in ['1,2,3','1,0,30,135','1740,877,1440,945','0,0,4000,2160']:
            with self.assertRaises(ValueError): arm_crop(3840,2160,crop)
        self.assertEqual(arm_crop(3840,2160,'1740,810,1440,1080'),(1740,810,1440,1080))


if __name__=='__main__': unittest.main()
