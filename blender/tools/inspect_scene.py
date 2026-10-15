"""Validate the authored rig and export current button bounds, without rendering."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
from types import SimpleNamespace

import bpy
from bpy_extras.object_utils import world_to_camera_view
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import configure_quality


REQUIRED_OBJECTS = (
    'Tonearm horizontal bearing barrel', 'Stylus diamond contact tip',
    'Tonearm - level tube with controlled end bend', 'Turntable - move this assembly',
    'Vinyl record - 12 inch', 'Paper center label', 'Wall - cream plaster',
    'Tabletop - amber wood', 'Inset brushed silver deck',
    'Placeholder LP sleeve - cream', 'Placeholder sleeve - top printed board',
    'Sleeve stand - continuous rounded metal frame', 'Jacket 5 - top printed board',
    'Speed selector knob', 'Speed dial index', 'Vinyl detail - dust and minor scratches',
)
CONTROLLER = 'Transport buttons - press state controller'
BUTTONS = {
    'play_pause': ('Play pause button', 'Play pause pressed'),
    'previous': ('Previous button', 'Previous pressed'),
    'next': ('Next button', 'Next pressed'),
}


def select_device(scene, requested):
    if requested != 'AUTO':
        configure_device(scene, requested)
        return requested
    for backend in ('OPTIX', 'CUDA'):
        try:
            configure_device(scene, backend)
            return backend
        except (RuntimeError, TypeError):
            pass
    configure_device(scene, 'CPU')
    return 'CPU'


def inspect(variant, device):
    if bpy.app.version != (4, 0, 2):
        raise ValueError('These bake tools are verified with Blender 4.0.2; use that version')
    scene = bpy.context.scene
    if not scene.camera or scene.use_nodes:
        raise ValueError('The bake needs an active fixed camera and no custom compositor')
    if scene.frame_current != 1:
        raise ValueError('Save the authored scene at frame 1 (the released-button baseline)')
    for name in (*REQUIRED_OBJECTS, CONTROLLER, *(value[0] for value in BUTTONS.values())):
        if name not in scene.objects:
            raise ValueError(f'Missing required rig object: {name}; retain rig names when editing')
    if len([obj for obj in scene.objects if obj.name.startswith('Placeholder sleeve - ')]) != 7:
        raise ValueError('Expected the seven authored sleeve meshes')
    if 'Cover visible' not in scene.objects['Placeholder LP sleeve - cream']:
        raise ValueError('Missing sleeve visibility control')
    if not scene.objects['Placeholder LP sleeve - cream']['Cover visible']:
        raise ValueError('Save the authored baseline with Cover visible enabled')
    controller = scene.objects[CONTROLLER]
    if any(key not in controller for _, key in BUTTONS.values()):
        raise ValueError('Missing transport button driver controls')
    if any(abs(float(controller[key])) > 1e-6 for _, key in BUTTONS.values()):
        raise ValueError('Save the authored baseline with transport buttons released')
    for image in bpy.data.images:
        if image.source not in {'FILE', 'MOVIE', 'SEQUENCE', 'TILED'}:
            continue
        if image.source != 'FILE':
            raise ValueError(f'Animated/tiled textures need a dedicated bake adapter: {image.name}')
        if not image.packed_file and not image.packed_files:
            raise ValueError(f'Pack external texture before baking: {image.name}')
    for library in bpy.data.libraries:
        raise ValueError(f'Make linked assets local before baking: {library.filepath}')
    for font in bpy.data.fonts:
        if font.filepath != '<builtin>' and not font.packed_file:
            raise ValueError(f'Pack external font before baking: {font.name}')
    for values in (bpy.data.sounds, bpy.data.movieclips, bpy.data.cache_files, bpy.data.volumes):
        for value in values:
            if value.filepath and not getattr(value, 'packed_file', None):
                raise ValueError(f'Unpacked media/cache needs a dedicated bake adapter: {value.name}')
    scene.render.engine = 'CYCLES'
    chosen_device = select_device(scene, device)
    configure_quality(scene, SimpleNamespace(samples=2048, noise_threshold=0.006,
                                             min_samples=64, color_depth='16'))
    scene.frame_set(1)
    # Match the existing scene's 16:9 aspect before projecting edited controls.
    scene.render.resolution_x, scene.render.resolution_y = 2560, 1440
    scene.render.resolution_percentage = 100
    for _, key in BUTTONS.values():
        controller[key] = 0.0
    controller.update_tag()
    bpy.context.view_layer.update()
    rectangles = {}
    for name, (object_name, _) in BUTTONS.items():
        obj = scene.objects[object_name].evaluated_get(bpy.context.evaluated_depsgraph_get())
        points = [world_to_camera_view(scene, scene.camera, obj.matrix_world @ Vector(corner))
                  for corner in obj.bound_box]
        if any(point.z <= 0 for point in points):
            raise ValueError(f'Button is behind the camera: {object_name}')
        # Conservative padding includes travel/contact-shadow surroundings;
        # the renderer quantizes the union to the exact output pixel grid.
        bounds = [min(p.x for p in points) - 16 / 2560,
                  1 - max(p.y for p in points) - 16 / 1440,
                  max(p.x for p in points) + 16 / 2560,
                  1 - min(p.y for p in points) + 16 / 1440]
        if not all(0 <= value <= 1 for value in bounds):
            raise ValueError(f'Button/shadow padding leaves the camera frame: {object_name}')
        rectangles[name] = bounds
    source = Path(bpy.data.filepath)
    return {'device': chosen_device, 'author': {'version': 3, 'controller': CONTROLLER,
        'modes': {variant: {'scene': str(source),
            'scene_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'camera': scene.camera.name, 'patch_rectangles_scene_uv': rectangles}}}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=('day', 'night'), required=True)
    parser.add_argument('--device', choices=('AUTO', 'CPU', 'CUDA', 'OPTIX'), default='AUTO')
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    print('TALIS_BAKE_SOURCE ' + json.dumps(inspect(args.variant, args.device)), flush=True)
