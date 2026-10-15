"""Bake a tiny stationary knob surface/ink response; never save the Blender source."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys
import bpy
from mathutils import Vector
from bpy_extras.object_utils import world_to_camera_view

sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from render_resolution import pixel_grid
from verify_scene_bake import valid_knob_crop

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--beauty', required=True)
p.add_argument('--output', required=True)
p.add_argument('--device', choices=('CPU', 'CUDA', 'OPTIX'), default='OPTIX')
p.add_argument('--samples', type=int, default=2048)
add_arguments(p)
a = p.parse_args(sys.argv[sys.argv.index('--') + 1:])
beauty = Path(a.beauty)
reference = json.loads(beauty.with_suffix('.json').read_text())
source = Path(bpy.data.filepath)
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
if digest(source) != reference['source_sha256'] or digest(beauty) != reference['image_sha256']:
    raise ValueError('Knob source/beauty mismatch; rebake for the shipped scene')
s = bpy.context.scene
s.frame_set(1)
if s.camera.name != reference['camera']:
    raise ValueError('Fixed camera must match the beauty')
knob = bpy.data.objects['Speed selector knob']
marker = bpy.data.objects['Speed dial index']
material = marker.data.materials[0]
bpy.context.view_layer.update()
radius = max(v.co.x for v in knob.data.vertices)
top = max(v.co.z for v in knob.data.vertices)
marker_local = [knob.matrix_world.inverted() @ marker.matrix_world @ v.co for v in marker.data.vertices]
marker_width = (max(v.x for v in marker_local)-min(v.x for v in marker_local))/(2*radius)
marker_inner, marker_outer = min(v.y for v in marker_local)/radius, max(v.y for v in marker_local)/radius
if abs(sum(v.x for v in marker_local)/len(marker_local)) > radius*0.001 or not 0 < marker_inner < marker_outer < 0.95:
    raise ValueError('Marker must be centred on the knob local +Y ray; update its calibrated geometry first')
# World-to-camera mapping of the actual knob's local top plane. Order has
# positive scene winding, allowing the same projective fit as album artwork.
def uv(x, y):
    q = world_to_camera_view(s, s.camera, knob.matrix_world @ Vector((x, y, top)))
    return [q.x, 1-q.y]
corners = [uv(-radius, radius), uv(radius, radius), uv(radius, -radius), uv(-radius, -radius)]
if sum((corners[(i+1)%4][0]-corners[i][0]) * (corners[(i+2)%4][1]-corners[(i+1)%4][1])
       - (corners[(i+1)%4][1]-corners[i][1]) * (corners[(i+2)%4][0]-corners[(i+1)%4][0]) for i in range(4)) < 0:
    corners = [corners[1], corners[0], corners[3], corners[2]]
w, h = reference['resolution']
points = [world_to_camera_view(s, s.camera, knob.matrix_world @ Vector(v)) for v in knob.bound_box]
gx, gy = (16,16) if (w,h)==(2560,1440) else pixel_grid(w,h)
left = max(0, math.floor((min(v.x for v in points)*w-10)/gx)*gx)
right = min(w, math.ceil((max(v.x for v in points)*w+10)/gx)*gx)
upper = max(0, math.floor(((1-max(v.y for v in points))*h-10)/gy)*gy)
lower = min(h, math.ceil(((1-min(v.y for v in points))*h+10)/gy)*gy)
if not valid_knob_crop([left, upper, right-left, lower-upper], [w,h]):
    raise ValueError('Planned knob crop exceeds the bounded runtime pixel/axis limits')
out = Path(a.output)
out.mkdir(parents=True, exist_ok=True)
s.render.engine = 'CYCLES'
configure_device(s, a.device)
configure_quality(s, a)
s.render.resolution_x, s.render.resolution_y = w, h
s.render.resolution_percentage = 100
s.render.use_border = s.render.use_crop_to_border = True
s.render.border_min_x, s.render.border_max_x = left/w, right/w
s.render.border_min_y, s.render.border_max_y = 1-lower/h, 1-upper/h
s.render.image_settings.file_format = 'PNG'
s.render.image_settings.color_mode = 'RGBA'
s.render.film_transparent = False
s.render.use_persistent_data = True
s.use_nodes = False
marker.hide_render = True
def render(name):
    s.render.filepath = str(out/name)
    bpy.ops.render.render(write_still=True)
    print('KNOB_BAKED', name, flush=True)
render('clean.png')
# Stationary light response for the actual warm-ivory marker material. Only
# cap faces change; body/rim remain fixed. Runtime samples this under the tick.
knob.data.materials.append(material)
cap_material = len(knob.data.materials)-1
for face in knob.data.polygons:
    if face.normal.z > 0.9:
        face.material_index = cap_material
render('ink.png')
if digest(source) != reference['source_sha256']:
    raise RuntimeError('Source was edited during the bake; discard these patches')
files = {name: {'file': name+'.png', 'sha256': digest(out/(name+'.png'))} for name in ('clean', 'ink')}
metadata = {
    'format_version': 1, 'width': w, 'height': h,
    'crop': [left, upper, right-left, lower-upper], 'corners': corners,
    'marker_width': marker_width,
    'marker_inner': marker_inner,
    'marker_outer': marker_outer,
    'source': str(source), 'source_sha256': reference['source_sha256'],
    'base_image_sha256': reference['image_sha256'], 'quality': quality_metadata(s), **files,
}
(out/'atlas.json').write_text(json.dumps(metadata, indent=2)+'\n')
print('KNOB_MANIFEST', json.dumps(metadata), flush=True)
