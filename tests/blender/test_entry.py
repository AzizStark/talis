"""Entry-point safety/command tests: no real Blender process or renders."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('talis_bake', REPO / 'blender/bake.py')
bake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bake)


def options(output, **changes):
    return SimpleNamespace(output=output, scene='all', resolution='all', device='CPU',
        blender='blender', check=False, resume=False, resume_from=None, full_arm_region=False,
        **changes)


class EntryTests(unittest.TestCase):
    def test_plan_covers_every_tier_mode_and_required_layer(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            args = options(root)
            for tier, (width, height) in bake.RESOLUTIONS.items():
                for variant in ('day', 'night'):
                    command = bake.bake_command(args, '/Blender app/blender', root, tier, variant, 'CPU')
                    self.assertEqual(command[0], sys.executable)
                    self.assertEqual(command[command.index('--width')+1], str(width))
                    self.assertEqual(command[command.index('--height')+1], str(height))
                    self.assertEqual(command[command.index('--arm-update-right')+1], str(600*width//2560))
                    self.assertIn('--with-knob', command)
                    self.assertEqual(command[command.index('--blender')+1], '/Blender app/blender')

    def test_snapshots_do_not_change_with_later_authoring_edits(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / 'author'; source.mkdir()
            for variant in ('day', 'night'):
                (source / f'{variant}.blend').write_bytes(variant.encode())
            output = root / 'output'; output.mkdir()
            with patch.object(bake, 'ROOT', source):
                hashes = bake.capture_sources(output, ['day', 'night'], False)
                (source / 'day.blend').write_bytes(b'later edit')
                self.assertEqual(bake.capture_sources(output, ['day', 'night'], True), hashes)
            self.assertEqual((source / 'day.blend').read_bytes(), b'later edit')

    def test_source_change_during_capture_fails_before_rendering(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); source = root / 'author'; source.mkdir()
            (source / 'day.blend').write_bytes(b'original')
            output = root / 'output'; output.mkdir()
            copy = bake.shutil.copyfile
            def edited(original, snapshot):
                copy(original, snapshot)
                original.write_bytes(b'new version')
            with patch.object(bake, 'ROOT', source), patch.object(bake.shutil, 'copyfile', edited):
                with self.assertRaisesRegex(ValueError, 'changed during capture'):
                    bake.capture_sources(output, ['day'], False)

    def test_live_assets_code_roots_and_ancestors_are_refused(self):
        for path in (REPO, REPO.parent, REPO/'assets/scenes', REPO/'blender', REPO/'blender/tools/cache'):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'separate bake directory'):
                bake.validate_output(path)
        self.assertEqual(bake.validate_output(REPO/'blender/output'), REPO/'blender/output')

    def test_output_lock_prevents_two_writers(self):
        with tempfile.TemporaryDirectory() as folder:
            with bake.output_lock(Path(folder)):
                with self.assertRaisesRegex(ValueError, 'Another bake'):
                    with bake.output_lock(Path(folder)):
                        self.fail('second writer acquired the lock')

    def test_inspection_uses_disabled_autoexec_and_preserves_spaces(self):
        command = bake.stage_command('/Blender app/blender', Path('/scene dir/day.blend'), 'day', 'CPU')
        self.assertEqual(command[0], '/Blender app/blender')
        self.assertIn('/scene dir/day.blend', command)
        self.assertIn('--disable-autoexec', command)
        with patch.object(bake.subprocess, 'run', return_value=SimpleNamespace(
                returncode=1, stdout='bad source', stderr='missing rig')):
            with self.assertRaisesRegex(ValueError, 'missing rig'):
                bake.inspect_source('blender', Path('scene.blend'), 'day', 'CPU')

    def test_full_pipeline_failure_is_not_marked_as_complete(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); author = root/'author'; author.mkdir()
            for variant in ('day', 'night'):
                (author/f'{variant}.blend').write_bytes(variant.encode())
            args = options(root/'output')
            inspected = {'device': 'CPU', 'author': {'modes': {v: {
                'patch_rectangles_scene_uv': {'cluster': [.35, .63, .42, .66]}}
                for v in ('day', 'night')}}}
            def run(command, **kwargs):
                if command[1].endswith('bake_scene_package.py'):
                    raise bake.subprocess.CalledProcessError(9, command)
                return SimpleNamespace(returncode=0)
            with patch.object(bake, 'ROOT', author), patch.object(bake.shutil, 'which', return_value='blender'), \
                    patch.object(bake.importlib.util, 'find_spec', return_value=True), \
                    patch.object(bake, 'inspect_source', return_value=inspected), \
                    patch.object(bake.subprocess, 'run', side_effect=run):
                with self.assertRaises(bake.subprocess.CalledProcessError):
                    bake.run(args)
            job = json.loads((args.output/'bake.json').read_text())
            self.assertEqual(job['completed'], [])
            self.assertEqual((author/'day.blend').read_bytes(), b'day')

    def test_complete_pairs_resume_without_rerendering_and_detect_source_corruption(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); author = root/'author'; author.mkdir()
            for variant in ('day', 'night'):
                (author/f'{variant}.blend').write_bytes(variant.encode())
            args = options(root/'output')
            inspected = {'device': 'CPU', 'author': {'modes': {v: {
                'patch_rectangles_scene_uv': {'cluster': [.35, .63, .42, .66]}}
                for v in ('day', 'night')}}}
            with patch.object(bake, 'ROOT', author), patch.object(bake.shutil, 'which', return_value='blender'), \
                    patch.object(bake.importlib.util, 'find_spec', return_value=True), \
                    patch.object(bake, 'inspect_source', return_value=inspected), \
                    patch.object(bake.subprocess, 'run', return_value=SimpleNamespace(returncode=0)) as run:
                bake.run(args)
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual(sum(c[1].endswith('bake_scene_package.py') for c in commands), 6)
                args.resume = True
                run.reset_mock()
                bake.run(args)
                self.assertEqual(run.call_count, 7, 'dependency check plus six hash-only verifications')
                self.assertTrue(all('bake_scene_package.py' not in ' '.join(call.args[0])
                                    for call in run.call_args_list))
                (args.output/'sources/day.blend').write_bytes(b'corrupt')
                run.reset_mock()
                with self.assertRaisesRegex(ValueError, 'captured source changed'):
                    bake.run(args)
                self.assertEqual(run.call_count, 1)

    def test_generated_preview_paths_are_absolute_and_preserve_quotes(self):
        with tempfile.TemporaryDirectory(prefix="talis 'bake-") as folder:
            output = Path(folder)
            bake.preview_config(output, '1440p')
            import tomllib
            config = tomllib.loads((output/'preview-1440p.toml').read_text())
            self.assertEqual(config['scene']['background'], str(output/'1440p/scenes/day/beauty.png'))
            self.assertEqual(config['scene']['night']['buttons'], str(output/'1440p/scenes/night/buttons/atlas.json'))


if __name__ == '__main__':
    unittest.main()
