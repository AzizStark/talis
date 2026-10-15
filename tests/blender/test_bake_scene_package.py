"""Snapshot/resume regressions using disposable fixtures, without Blender."""
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import bake_scene_package as bake


class BakeTests(unittest.TestCase):
    def test_1080p_dimensions_and_crop_do_not_use_1440p_pixel_units(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder); source=root/'author.blend'; source.write_bytes(b'scene')
            manifest=root/'manifest.json'
            manifest.write_text(json.dumps({'modes':{'day':{'scene':str(source),'scene_sha256':bake.digest(source)}}}))
            argv=['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                  '--variant','day','--width','1920','--height','1080','--arm-update-right','450','--with-knob']
            with patch.object(sys,'argv',argv),patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                bake.main()
            commands=[call.args[0] for call in run.call_args_list]
            for script in ('render_beauty.py','render_transport_buttons.py'):
                command=next(c for c in commands if any(item.endswith(script) for item in c))
                self.assertEqual(command[command.index('--width')+1],'1920')
                self.assertEqual(command[command.index('--height')+1],'1080')
            poses=next(c for c in commands if any(item.endswith('render_arm_poses.py') for item in c))
            self.assertEqual(poses[poses.index('--crop')+1],'870,405,720,540')
            region=next(c for c in commands if any(item.endswith('prepare_arm_region.py') for item in c))
            self.assertEqual(region[-2:],['--right','450'])
            self.assertTrue(any('prepare_knob.py' in ' '.join(c) for c in commands))

    def test_4k_dimensions_reach_beauty_buttons_and_scaled_arm_crop(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder); source=root/'author.blend'; source.write_bytes(b'scene')
            manifest=root/'manifest.json'
            manifest.write_text(json.dumps({'modes':{'day':{'scene':str(source),'scene_sha256':bake.digest(source)}}}))
            argv=['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                  '--variant','day','--width','3840','--height','2160','--arm-update-right','900']
            with patch.object(sys,'argv',argv),patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                bake.main()
            commands=[call.args[0] for call in run.call_args_list]
            for script in ('render_beauty.py','render_transport_buttons.py'):
                command=next(c for c in commands if any(item.endswith(script) for item in c))
                self.assertEqual(command[command.index('--width')+1],'3840')
                self.assertEqual(command[command.index('--height')+1],'2160')
            poses=next(c for c in commands if any(item.endswith('render_arm_poses.py') for item in c))
            self.assertEqual(poses[poses.index('--crop')+1],'1740,810,1440,1080')
            region=next(c for c in commands if any(item.endswith('prepare_arm_region.py') for item in c))
            self.assertEqual(region[-2:],['--right','900'])
            (root/'stage/scenes/day/beauty.json').write_text(json.dumps({'resolution':[3840,2160]}))
            with patch.object(sys,'argv',argv+['--resume-from','record']),patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                bake.main()
                self.assertGreater(run.call_count,0)
            with patch.object(sys,'argv',['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                    '--variant','day','--resume-from','record']),patch.object(bake.subprocess,'run') as run:
                with self.assertRaisesRegex(ValueError,'Resume resolution differs'): bake.main()
                run.assert_not_called()

    def test_new_stages_use_explicit_day_and_night_roots(self):
        for variant in ('day','night'):
            with tempfile.TemporaryDirectory() as folder:
                root=Path(folder);source=root/'author.blend';source.write_bytes(b'scene')
                manifest=root/'manifest.json'
                manifest.write_text(json.dumps({'modes':{variant:{'scene':str(source),'scene_sha256':bake.digest(source)}}}))
                argv=['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                      '--variant',variant,'--resume-from','record','--with-knob']
                with patch.object(sys,'argv',argv),patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                    bake.main()
                expected=str(root/'stage/scenes'/variant/'beauty.png')
                self.assertIn(expected,run.call_args_list[0].args[0])
                self.assertEqual(json.loads((root/'stage'/(variant+'-status.json')).read_text())['package'],str(root/'stage/scenes'/variant))
                before=run.call_count
                (root/'stage/scenes/beauty.json').write_text('{}')
                with patch.object(sys,'argv',argv),self.assertRaisesRegex(ValueError,'Legacy flat stage layout'):
                    bake.main()
                self.assertEqual(run.call_count,before)

    def test_verify_resume_and_explicit_no_packing_do_not_repack(self):
        for resume, extra in [('verify',[]),('record',['--no-pack-arm'])]:
            with tempfile.TemporaryDirectory() as folder:
                root=Path(folder);source=root/'author.blend';source.write_bytes(b'scene')
                manifest=root/'manifest.json'
                manifest.write_text(json.dumps({'modes':{'day':{'scene':str(source),'scene_sha256':bake.digest(source)}}}))
                argv=['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                      '--variant','day','--resume-from',resume,*extra]
                with patch.object(sys,'argv',argv),patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                    bake.main()
                self.assertFalse(any('pack_arm_assets.py' in ' '.join(c.args[0]) for c in run.call_args_list))
                self.assertEqual(run.call_count,1 if resume=='verify' else 5)
    def test_edge_resume_refreshes_display_boundary_and_previous_cover_calibration(self):
        for right in (None,600):
            with tempfile.TemporaryDirectory(prefix='talis-bake-test-') as folder:
                root=Path(folder)
                source=root/'author.blend'; source.write_bytes(b'scene')
                manifest=root/'manifest.json'
                manifest.write_text(json.dumps({'modes':{'day':{'scene':str(source),'scene_sha256':bake.digest(source)}}}))
                argv=['bake','--author-manifest',str(manifest),'--stage-root',str(root/'stage'),
                      '--variant','day','--resume-from','edges']
                if right is not None: argv += ['--arm-update-right',str(right)]
                with patch.object(sys,'argv',argv), patch.object(bake.subprocess,'run',return_value=SimpleNamespace(returncode=0)) as run:
                    bake.main()
                commands=[call.args[0] for call in run.call_args_list]
                region=next(cmd for cmd in commands if any('prepare_arm_region.py' in item for item in cmd))
                self.assertEqual(region[-2:],['--right',str(right or 960)])
                stack=next(cmd for cmd in commands if 'stack' in cmd)
                self.assertIn('--response-kind',stack)
                self.assertIn('Jacket 5 - top printed board',stack)
                self.assertTrue(any('prepare_empty_stand.py' in ' '.join(cmd) for cmd in commands))
                self.assertTrue(any('pack_arm_assets.py' in ' '.join(cmd) for cmd in commands))
                self.assertEqual(len(commands),11)

    def test_resume_uses_immutable_capture_not_later_authoring_edits(self):
        with tempfile.TemporaryDirectory(prefix='talis-bake-test-') as folder:
            root=Path(folder)
            source=root/'author.blend'
            source.write_bytes(b'original scene')
            manifest=root/'manifest.json'
            manifest.write_text(json.dumps({'modes':{'day':{
                'scene':str(source),'scene_sha256':bake.digest(source)}}}))
            stage=root/'stage'
            argv=['bake','--author-manifest',str(manifest),'--stage-root',str(stage),
                  '--variant','day','--resume-from','record']
            with patch.object(sys,'argv',argv), patch.object(bake.subprocess,'run',
                    return_value=SimpleNamespace(returncode=0)) as run:
                bake.main()
                self.assertEqual(run.call_count,6)
                self.assertIn('prepare_record_lighting.py',' '.join(run.call_args_list[0].args[0]))
                source.write_bytes(b'new authoring edits')
                run.reset_mock()
                bake.main()
                self.assertEqual(run.call_count,6)
            self.assertEqual((stage/'sources/day.blend').read_bytes(),b'original scene')
            self.assertEqual(source.read_bytes(),b'new authoring edits')
            self.assertFalse(json.loads((stage/'day-status.json').read_text())[
                'authoring_source_still_matches_snapshot'])
            (stage/'sources/day.blend').write_bytes(b'corrupt snapshot')
            with patch.object(sys,'argv',argv), self.assertRaisesRegex(ValueError,'do not match'):
                bake.main()
            self.assertEqual((stage/'sources/day.blend').read_bytes(),b'corrupt snapshot')


if __name__=='__main__': unittest.main()
