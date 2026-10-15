"""Packing regressions with disposable 16-bit masters and real encoded fields."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import cv2
import numpy as np

from pack_arm_assets import pack, digest, write_json, packed_width, install_staged, vector_max


def fixture(root):
    (root / 'arm-poses').mkdir(parents=True)
    (root / 'arm-flow').mkdir()
    (root / 'beauty.png').write_bytes(b'beauty')
    frames, hashes = [], []
    for i in range(3):
        name = f'pose-{i}.png'
        data = np.arange(32*128*3, dtype=np.uint16).reshape(32,128,3) + i
        cv2.imwrite(str(root/'arm-poses'/name),data)
        hashes.append(digest(root/'arm-poses'/name))
        frames.append({'file':name,'progress':i/2,'angle_degrees':float(i)})
    poses = {'format_version':1,'source_sha256':'source','base_image_sha256':digest(root/'beauty.png'),
             'crop':[20,10,128,32],'frames':frames}
    write_json(root/'arm-poses/atlas.json',poses)
    ph=digest(root/'arm-poses/atlas.json')
    write_json(root/'arm-guide.json',{'format_version':1,'source_sha256':'source',
               'pose_manifest_sha256':ph,'projection':[[1,2,3,4],[5,6,7,8],[9,10,11,12]]})
    gh=digest(root/'arm-guide.json')
    def save(name,data):
        cv2.imwrite(str(root/'arm-flow'/name),data)
        return {'file':name,'sha256':digest(root/'arm-flow'/name)}
    fields=[save(f'flow-{i}.png',np.full((16,64,4),33024,dtype=np.uint16)) for i in range(2)]
    masks=[save(f'mask-{i}.png',np.full((32,128),i,dtype=np.uint8)) for i in range(3)]
    bg=save('background.png',np.full((32,128,4),127,dtype=np.uint8))
    flow={'format_version':1,'width':64,'height':16,'crop_width':128,'crop_height':32,
          'encoding':'rgba16_unorm_bias32768_scale256','pose_manifest_sha256':ph,
          'pose_images_sha256':hashes,'tube_guide_sha256':gh,'pairs':fields,
          'edge_layers':{'guide_sha256':gh,'masks':masks,'background':bg}}
    write_json(root/'arm-flow/atlas.json',flow)
    write_json(root/'arm-poses/update-region.json',{'format_version':1,'crop':[20,10,64,32],
               'pose_manifest_sha256':ph,'flow_manifest_sha256':digest(root/'arm-flow/atlas.json')})


class PackingTests(unittest.TestCase):
    def test_failed_staged_install_rolls_back_and_live_assets_are_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'source';output=root/'output';fixture(source)
            originals={str(p.relative_to(source)):digest(p) for p in source.rglob('*') if p.is_file()}
            pack(source,output)
            rename=Path.rename
            def fail_flow(path,target):
                if path==output/'arm-flow': raise OSError('simulated interrupted installation')
                return rename(path,target)
            with patch.object(Path,'rename',fail_flow),self.assertRaisesRegex(OSError,'simulated'):
                install_staged(source,output)
            self.assertEqual(originals,{str(p.relative_to(source)):digest(p) for p in source.rglob('*') if p.is_file()})
            with self.assertRaisesRegex(ValueError,'complete-pair publisher'):
                install_staged(Path(__file__).resolve().parents[2]/'assets/scenes',output)
            with self.assertRaisesRegex(ValueError,'backups are read-only'):
                install_staged(root/'scene-refresh-backup.test/previous-scenes',output)
    def test_lossless_pixels_grid_hashes_projection_and_cadence(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder); source=root/'source'; output=root/'output'
            fixture(source)
            originals={str(p.relative_to(source)):digest(p) for p in source.rglob('*') if p.is_file()}
            report=pack(source,output)
            self.assertEqual(report['packed_crop'],[20,10,96,32])
            self.assertEqual(report['sampling_margin'],16)
            flow=json.loads((output/'arm-flow/atlas.json').read_text())
            guide=json.loads((output/'arm-guide.json').read_text())
            region=json.loads((output/'arm-poses/update-region.json').read_text())
            self.assertEqual(flow['width'],48)
            self.assertEqual(region['crop'],[20,10,64,32])
            self.assertEqual(flow['pose_manifest_sha256'],digest(output/'arm-poses/atlas.json'))
            self.assertEqual(guide['pose_manifest_sha256'],flow['pose_manifest_sha256'])
            self.assertEqual(flow['tube_guide_sha256'],digest(output/'arm-guide.json'))
            self.assertEqual(flow['edge_layers']['guide_sha256'],flow['tube_guide_sha256'])
            self.assertEqual(region['flow_manifest_sha256'],digest(output/'arm-flow/atlas.json'))
            self.assertEqual(guide['projection'],json.loads((source/'arm-guide.json').read_text())['projection'])
            for p in output.rglob('*.png'):
                old=cv2.imread(str(source/p.relative_to(output)),-1)
                new=cv2.imread(str(p),-1)
                self.assertTrue(np.array_equal(new,old[:,:new.shape[1]]))
            self.assertEqual(originals,{str(p.relative_to(source)):digest(p) for p in source.rglob('*') if p.is_file()})
            with self.assertRaisesRegex(ValueError,'existing data'):
                pack(source,output)
            backup=install_staged(source,output)
            self.assertEqual(json.loads((source/'arm-poses/atlas.json').read_text())['crop'][2],96)
            self.assertEqual(json.loads((backup/'arm-poses/atlas.json').read_text())['crop'][2],128)

    def test_extremum_outside_display_region_keeps_cadence_conservative(self):
        fields=[np.full((16,64,4),32768,dtype=np.uint16)]
        fields[0][:,-1,:]=34048
        keep,_,_=packed_width(128,64,2,fields)
        self.assertEqual(keep,128)
        self.assertEqual(packed_width(128,128,2,fields)[0],128)
        # Asymmetric vectors catch accidental BGRA/RGBA component pairing.
        example=(np.array([[[12,4,3,5]]],dtype=np.uint16)*256+32768)
        self.assertEqual(vector_max(example),13.0)
        field=np.full((16,128,4),32768,dtype=np.uint16)
        field[:,0]=np.array([0,0,12,12],dtype=np.uint16)*256+32768
        field[:,-1]=np.array([0,10,10,0],dtype=np.uint16)*256+32768
        self.assertEqual(packed_width(256,64,2,[field])[0],256)

    def test_stale_or_unsupported_sources_do_not_produce_output(self):
        for relative,key,value in [('arm-poses/update-region.json','flow_manifest_sha256','stale'),
                                   ('arm-flow/atlas.json','width',63)]:
            with tempfile.TemporaryDirectory() as folder:
                root=Path(folder); source=root/'source'; output=root/'output';fixture(source)
                target=source/relative; data=json.loads(target.read_text()); data[key]=value;write_json(target,data)
                with self.assertRaises(ValueError): pack(source,output)
                self.assertFalse(output.exists())
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);fixture(root/'source')
            (root/'source/arm-poses/pose-1.png').write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError,'Stale/corrupt'):
                pack(root/'source',root/'output')
            self.assertFalse((root/'output').exists())


if __name__=='__main__': unittest.main()
