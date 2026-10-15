"""Stage a complete matched scene package from a byte-identical source snapshot.

Never publishes automatically or saves an authoring scene. Keep the stage root
for native preview tests and as recoverable bake provenance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tempfile
from scene_package_contract import BAKE_STAGES
from render_resolution import arm_crop, validate_resolution

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--author-manifest',type=Path,required=True)
    parser.add_argument('--stage-root',type=Path,required=True)
    parser.add_argument('--variant',choices=('day','night'),required=True)
    parser.add_argument('--blender',default='blender',help='Blender executable selected by the entry point')
    parser.add_argument('--device',choices=('CPU','CUDA','OPTIX'),default='OPTIX')
    parser.add_argument('--width',type=int,default=2560)
    parser.add_argument('--height',type=int,default=1440)
    parser.add_argument('--with-knob',action='store_true',help='Include the optional volume-knob calibration')
    parser.add_argument('--pack-arm',action=argparse.BooleanOptionalAction,default=True,
                        help='Losslessly pack runtime arm textures after preparing wide masters (default on)')
    parser.add_argument('--stack-object',default='Jacket 5 - top printed board')
    parser.add_argument('--arm-update-right',type=int,
                        help='Optional visually verified right edge in crop-local pixels; full arm ROI is the safe default')
    stages=BAKE_STAGES
    parser.add_argument('--resume-from',choices=stages,
                        help='Resume a private staged bake; final verification still checks every asset')
    parser.add_argument('--crop',
                        help='Output-resolution x,y,width,height pixels; default scales the verified camera/rig crop outward')
    args=parser.parse_args()
    validate_resolution(args.width,args.height)
    crop=arm_crop(args.width,args.height,args.crop)
    right=args.arm_update_right if args.arm_update_right is not None else crop[2]
    if not 32 < right <= crop[2]:
        raise ValueError('Arm update right edge must fit the output crop')
    tools=Path(__file__).resolve().parent
    repo=tools.parents[1]
    root=args.stage_root.resolve()
    if root == repo or root.is_relative_to(repo/'assets') or repo.is_relative_to(root):
        raise ValueError('Bake into a separate staging directory, never the live assets or repository root')
    if (root/'scenes/beauty.json').exists() or (root/'scenes/beauty.png').exists():
        raise ValueError('Legacy flat stage layout: use a fresh stage root for scenes/day and scenes/night')
    source_dir=root/'sources'
    source_dir.mkdir(parents=True,exist_ok=True)
    captured_manifest=source_dir/(args.variant+'-author-manifest.json')
    # A long day bake must not accidentally pick up newer, unmatched night
    # edits. An explicitly captured source/manifest always wins on later stages.
    author=json.loads((captured_manifest if captured_manifest.exists() else args.author_manifest).read_text())
    info=author['modes'][args.variant]
    source=Path(info['scene'])
    snapshot=source_dir/(args.variant+'.blend')
    if snapshot.exists():
        if not captured_manifest.exists() or digest(snapshot)!=info['scene_sha256']:
            raise ValueError('Existing snapshot and captured manifest do not match')
    else:
        if digest(source)!=info['scene_sha256']:
            raise ValueError('Authoring scene and manifest hashes differ')
        shutil.copy2(source,snapshot)
        if digest(snapshot)!=info['scene_sha256'] or digest(source)!=info['scene_sha256']:
            raise ValueError('Source changed during snapshot copy')
        captured_manifest.write_text(json.dumps(author,indent=2)+'\n')
    out=root/'scenes'/args.variant
    if args.resume_from and (out/'beauty.json').exists():
        if json.loads((out/'beauty.json').read_text())['resolution'] != [args.width,args.height]:
            raise ValueError('Resume resolution differs from the staged beauty; use a fresh stage')
    out.mkdir(parents=True,exist_ok=True)
    logs=root/'logs'
    logs.mkdir(exist_ok=True)
    start=time.monotonic()
    status=root/(args.variant+'-status.json')
    def run(label,command):
        if args.resume_from and stages.index(label)<stages.index(args.resume_from):
            return
        print('TALIS_PACKAGE_STAGE',args.variant,label,flush=True)
        data={'variant':args.variant,'stage':label,'state':'running',
              'resolution':[args.width,args.height],'elapsed_seconds':time.monotonic()-start}
        status.write_text(json.dumps(data,indent=2)+'\n')
        with (logs/(args.variant+'-'+label+'.log')).open('w') as log:
            result=subprocess.run(command,cwd=repo,stdout=log,stderr=subprocess.STDOUT)
        if result.returncode:
            data['state']='failed'
            status.write_text(json.dumps(data,indent=2)+'\n')
            raise RuntimeError(f'{args.variant} {label} failed; see {logs}/{args.variant}-{label}.log')
    def blender(label,script,*arguments):
        run(label,[args.blender,'--background','--factory-startup',
                   '--disable-autoexec','--python-exit-code','1',str(snapshot),'--python',
                   str(tools/script),'--',*map(str,arguments)])
    quality=['--samples','2048','--noise-threshold','0.006','--min-samples','64','--device',args.device]
    blender('beauty','render_beauty.py',out/'beauty.png','--width',args.width,'--height',args.height,*quality)
    blender('arm-poses','render_arm_poses.py',out/'arm-poses','--crop',','.join(map(str,crop)),*quality)
    blender('arm-guide','export_arm_guide.py','--poses',out/'arm-poses/atlas.json','--output',out/'arm-guide.json')
    python=sys.executable
    run('flow',[python,str(tools/'prepare_arm_flow.py'),'--poses',str(out/'arm-poses/atlas.json'),
                '--guide',str(out/'arm-guide.json'),'--output',str(out/'arm-flow')])
    run('edges',[python,str(tools/'prepare_arm_edges.py'),'--poses',str(out/'arm-poses/atlas.json'),
                 '--guide',str(out/'arm-guide.json'),'--atlas',str(out/'arm-flow/atlas.json')])
    # Refresh this hash-bound metadata even for the safe full-width default;
    # otherwise an earlier trimmed region can survive a resumed pose/flow bake.
    run('arm-region',[python,str(tools/'prepare_arm_region.py'),str(out/'arm-poses/atlas.json'),
                      str(out/'arm-flow/atlas.json'),'--right',str(right)])
    blender('sleeve','prepare_sleeve.py',out/'sleeve','--beauty',out/'beauty.png',*quality)
    blender('stack-sleeve','prepare_sleeve.py',out/'stack-sleeve','--beauty',out/'beauty.png',
            '--object',args.stack_object,'--response-kind','stack',*quality)
    blender('empty-stand','prepare_empty_stand.py',out/'empty-stand','--beauty',out/'beauty.png',*quality)
    blender('record','prepare_record_lighting.py',out/'record-lighting','--beauty',out/'beauty.png',
            '--poses',out/'arm-poses/atlas.json',*quality)
    blender('vinyl','prepare_vinyl_detail.py',out/'vinyl-detail','--beauty',out/'beauty.png',
            '--poses',out/'arm-poses/atlas.json','--lighting',out/'record-lighting/atlas.json',*quality)
    button_bake=root/(args.variant+'-button-bake')
    blender('buttons','render_transport_buttons.py','--author-manifest',source_dir/(args.variant+'-author-manifest.json'),
            '--variant',args.variant,'--output',button_bake,'--width',args.width,'--height',args.height,*quality)
    run('button-package',[python,str(tools/'package_transport_buttons.py'),str(button_bake),
                          str(out/'beauty.png'),str(out/'buttons')])
    # Preserve the optional volume-knob feature when replacing an existing
    # complete pair: its own tiny patch must match the newly baked beauty.
    if args.with_knob:
        blender('knob','prepare_knob.py','--beauty',out/'beauty.png','--output',out/'knob',*quality)
    if args.pack_arm and (not args.resume_from or stages.index(args.resume_from)<=stages.index('arm-pack')):
        packed=Path(tempfile.mkdtemp(prefix=args.variant+'-arm-pack-',dir=root))/'packed'
        run('arm-pack',[python,str(tools/'pack_arm_assets.py'),'--scene',str(out),
                        '--output',str(packed),'--install-staged'])
    run('verify',[python,str(tools/'verify_scene_bake.py'),'--scenes',str(out),'--source',str(snapshot)])
    if digest(snapshot)!=info['scene_sha256']:
        raise RuntimeError('Source snapshot changed during preparation')
    status.write_text(json.dumps({'variant':args.variant,'state':'verified','source':str(source),
        'resolution':[args.width,args.height],
        'source_sha256':digest(snapshot),'authoring_source_still_matches_snapshot':source.is_file() and digest(source)==digest(snapshot),
        'package':str(out),'elapsed_seconds':time.monotonic()-start},indent=2)+'\n')
    print('TALIS_PACKAGE_READY',args.variant,str(out),flush=True)

if __name__=='__main__': main()
