#!/usr/bin/env python3
"""Rebake Talis's complete scene layers without modifying sources or live assets.

Requires Blender 4.0.2 and the Python packages in blender/requirements.txt.
Default: both lighting scenes at 1080p, 1440p and 4K, production quality.
Use --check first: it validates sources/dependencies but never renders.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import importlib.util
import json
from pathlib import Path
import os
import shutil
import subprocess
import sys

from tools.render_resolution import button_crop
from tools.scene_package_contract import BAKE_STAGES


ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
TOOLS = ROOT / 'tools'
RESOLUTIONS = {'1080p': (1920, 1080), '1440p': (2560, 1440), '4k': (3840, 2160)}


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def selection(value, choices):
    return list(choices) if value == 'all' else [value]


def stage_command(blender, source, variant, device):
    return [blender, '--background', '--factory-startup', '--disable-autoexec',
            '--python-exit-code', '1', str(source), '--python',
            str(TOOLS / 'inspect_scene.py'), '--', '--variant', variant, '--device', device]


def inspect_source(blender, source, variant, device):
    completed = subprocess.run(stage_command(blender, source, variant, device),
                               capture_output=True, text=True)
    marker = 'TALIS_BAKE_SOURCE '
    records = [line[len(marker):] for line in completed.stdout.splitlines() if line.startswith(marker)]
    if completed.returncode or len(records) != 1:
        raise ValueError(f'{variant} source validation failed:\n{completed.stdout}\n{completed.stderr}')
    return json.loads(records[0])


def validate_output(output):
    output = output.resolve()
    if (REPO.is_relative_to(output)
            or output.is_relative_to(REPO) and not output.is_relative_to(ROOT / 'output')):
        raise ValueError('Output must be a separate bake directory, not source code or live assets')
    return output


def validate_bounds(result, variant, tiers):
    rectangles = list(result['author']['modes'][variant]['patch_rectangles_scene_uv'].values())
    for tier in tiers:
        button_crop(*RESOLUTIONS[tier], rectangles)


@contextmanager
def output_lock(output):
    output.mkdir(parents=True, exist_ok=True)
    with (output / '.bake.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError('Another bake is using this output directory') from error
        yield


def save_json(path, data):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(data, indent=2) + '\n')
    temporary.replace(path)


def capture_sources(output, variants, resume):
    sources = output / 'sources'
    sources.mkdir(exist_ok=True)
    hashes = {}
    for variant in variants:
        original, snapshot = ROOT / f'{variant}.blend', sources / f'{variant}.blend'
        if resume:
            if not snapshot.is_file():
                raise ValueError(f'Missing captured source: {snapshot}')
        else:
            before = digest(original)
            shutil.copyfile(original, snapshot)
            if digest(original) != before or digest(snapshot) != before:
                raise ValueError(f'{original} changed during capture; use a new output directory')
        hashes[variant] = digest(snapshot)
    return hashes


def bake_command(args, blender, output, tier, variant, device):
    width, height = RESOLUTIONS[tier]
    command = [sys.executable, str(TOOLS / 'bake_scene_package.py'),
        '--author-manifest', str(output / 'sources' / f'{variant}-author-manifest.json'),
        '--stage-root', str(output / tier), '--variant', variant, '--blender', blender,
        '--width', str(width), '--height', str(height), '--device', device, '--with-knob']
    if not args.full_arm_region:
        # Preserve the artist-approved right cut; geometry/flow validation must
        # pass again after every bake. Changed rigs can request the full ROI.
        command += ['--arm-update-right', str(600 * width // 2560)]
    if args.resume_from:
        command += ['--resume-from', args.resume_from]
    return command


def preview_config(output, tier):
    """Absolute paths make previews independent of the shell working directory."""
    names = {'background': 'beauty.png', 'arm_poses': 'arm-poses/atlas.json',
             'arm_flow': 'arm-flow/atlas.json', 'sleeve': 'sleeve/atlas.json',
             'record_lighting': 'record-lighting/atlas.json',
             'vinyl_detail': 'vinyl-detail/atlas.json', 'buttons': 'buttons/atlas.json'}
    lines = ['[scene]', 'mode = "arm"', 'resolution = "automatic"']
    for variant in ('day', 'night'):
        if variant == 'night':
            lines += ['', '[scene.night]']
        for key, name in names.items():
            # JSON string quoting is compatible with TOML basic strings.
            lines.append(f'{key} = {json.dumps(str(output / tier / "scenes" / variant / name))}')
    lines += ['', '[render]', 'arm_interpolation = true', '']
    (output / f'preview-{tier}.toml').write_text('\n'.join(lines))


def run(args):
    variants = selection(args.scene, ('day', 'night'))
    tiers = selection(args.resolution, RESOLUTIONS)
    if args.resume_from and (len(variants) != 1 or len(tiers) != 1):
        raise ValueError('--resume-from requires exactly one --scene and --resolution')
    blender = shutil.which(args.blender)
    if not blender:
        raise ValueError('Blender not found; install 4.0.2 or pass --blender /path/to/blender')
    missing = [package for package in ('numpy', 'cv2', 'PIL') if importlib.util.find_spec(package) is None]
    if missing:
        raise ValueError('Missing Python packages: ' + ', '.join(missing)
                         + '\nInstall with: python -m pip install -r blender/requirements.txt')
    # Test real imports now, not hours into a render after loading native wheels.
    subprocess.run([sys.executable, '-c', 'import numpy, cv2, PIL'], check=True)
    if args.check:
        for variant in variants:
            result = inspect_source(blender, ROOT / f'{variant}.blend', variant, args.device)
            validate_bounds(result, variant, tiers)
            print(f'PASS {variant}: packed dependencies and rig; {result["device"]} available', flush=True)
        print('No renders started. Planned tiers: ' + ', '.join(tiers))
        return
    output = validate_output(args.output)
    with output_lock(output):
        job_path = output / 'bake.json'
        if job_path.exists() and not args.resume:
            raise ValueError('Output already contains a bake; use a new --output or --resume')
        if args.resume and not job_path.exists():
            raise ValueError('--resume requires an existing bake.json')
        settings = {'scenes': variants, 'resolutions': tiers, 'device': args.device,
                    'full_arm_region': args.full_arm_region}
        if args.resume:
            job = json.loads(job_path.read_text())
            # A targeted stage repair may select a subset of the captured job.
            expected = job['settings']
            if (not set(variants) <= set(expected['scenes'])
                    or not set(tiers) <= set(expected['resolutions'])
                    or any(settings[key] != expected[key] for key in ('device', 'full_arm_region'))):
                raise ValueError('Resume settings differ from the captured bake')
        else:
            unexpected = [path for path in output.iterdir() if path.name != '.bake.lock']
            if unexpected:
                raise ValueError('New bakes require an empty output directory')
            job = {'version': 1, 'settings': settings, 'source_hashes': {}, 'completed': []}
            save_json(job_path, job)
        hashes = capture_sources(output, variants, args.resume)
        if args.resume and any(job['source_hashes'].get(key) != value for key, value in hashes.items()):
            raise ValueError('A captured source changed; do not resume this bake')
        job['source_hashes'].update(hashes)
        save_json(job_path, job)
        devices = {}
        # Validate every selected mode before spending time rendering any tier.
        for variant in variants:
            snapshot = output / 'sources' / f'{variant}.blend'
            result = inspect_source(blender, snapshot, variant, args.device)
            validate_bounds(result, variant, tiers)
            devices[variant] = result['device']
            save_json(output / 'sources' / f'{variant}-author-manifest.json', result['author'])
            print(f'Ready {variant}: {devices[variant]}; captured source {hashes[variant][:12]}', flush=True)
        for tier in tiers:
            for variant in variants:
                key = f'{tier}/{variant}'
                if key in job['completed'] and not args.resume_from:
                    subprocess.run([sys.executable, str(TOOLS / 'verify_scene_bake.py'),
                        '--scenes', str(output / tier / 'scenes' / variant),
                        '--source', str(output / 'sources' / f'{variant}.blend')],
                        cwd=REPO, check=True)
                    print(f'Reverified without rendering: {key}', flush=True)
                    continue
                if key in job['completed']:
                    job['completed'].remove(key)
                    save_json(job_path, job)
                subprocess.run(bake_command(args, blender, output, tier, variant, devices[variant]),
                               cwd=REPO, check=True)
                if key not in job['completed']:
                    job['completed'].append(key)
                save_json(job_path, job)
            if all(f'{tier}/{variant}' in job['completed'] for variant in ('day', 'night')):
                preview_config(output, tier)
        print(f'Verified bake output: {output}\nLive assets and authoring scenes were not modified.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--output', type=Path, default=ROOT / 'output', help='Fresh output directory; use a roomy disk')
    parser.add_argument('--resolution', choices=(*RESOLUTIONS, 'all'), default='all')
    parser.add_argument('--scene', choices=('day', 'night', 'all'), default='all')
    parser.add_argument('--device', choices=('AUTO', 'CPU', 'CUDA', 'OPTIX'), default='AUTO',
                        help='AUTO prefers OptiX, then CUDA, then CPU')
    parser.add_argument('--blender', default=os.environ.get('BLENDER_BIN', 'blender'))
    parser.add_argument('--check', action='store_true', help='Validate dependencies and source rigs; never render')
    parser.add_argument('--resume', action='store_true', help='Keep captured sources and skip verified scene/tier pairs')
    parser.add_argument('--resume-from', choices=BAKE_STAGES,
                        help='Repair one scene/tier from this stage; requires --resume')
    parser.add_argument('--full-arm-region', action='store_true', help='Do not apply the current rig\'s artist-approved right cut')
    args = parser.parse_args(argv)
    if args.resume_from and not args.resume:
        parser.error('--resume-from requires --resume')
    try:
        run(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Bake failed: {error}\n')
    except KeyboardInterrupt:
        parser.exit(130, 'Bake interrupted. Sources/live assets are unchanged; inspect stage logs before resuming.\n')


if __name__ == '__main__':
    main()
