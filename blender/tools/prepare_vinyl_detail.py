"""Export authored rotating surface marks and fixed diffuse response; source is never saved.

Runtime replaces the baked zero-angle decal with a rotated decal, rather than
rotating the beauty (and its highlights/shadows). The correction is local and
attenuated beneath the existing arm shadow. It is a calibrated 2.5D illusion,
not a rotating BRDF or a newly evaluated shadow.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from bpy_extras.object_utils import world_to_camera_view

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_sleeve import digest, pixels
from render_device import configure_device
from render_quality import add_arguments, configure_quality, quality_metadata
from scene_signature import signature, detail_response_signature


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--beauty', type=Path, required=True)
    parser.add_argument('--poses', type=Path, required=True)
    parser.add_argument('--lighting', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=2048)
    parser.add_argument('--device', choices=('CPU', 'CUDA', 'OPTIX'), default='CPU')
    parser.add_argument('--references', action='store_true')
    parser.add_argument('--references-only', action='store_true', help='Refresh only independent reference renders in a staged package')
    parser.add_argument('--refresh-from',type=Path,help='Reuse an existing response after proving only texture/opacity changed')
    parser.add_argument('--baseline-source',type=Path,help='Original source snapshot matching the existing scene/response')
    parser.add_argument('--overlay-only',action='store_true',help='Import only texture/opacity; explicitly retain the original scene when other source objects changed')
    add_arguments(parser)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.references_only and not args.references:
        parser.error('--references-only requires --references')
    if bool(args.refresh_from)!=bool(args.baseline_source):
        parser.error('--refresh-from and --baseline-source must be supplied together')
    if args.overlay_only and not args.refresh_from:
        parser.error('--overlay-only requires --refresh-from and --baseline-source')
    source = Path(bpy.data.filepath)
    source_hash, beauty_hash = digest(source), digest(args.beauty)
    baseline = json.loads(args.beauty.with_suffix('.json').read_text())
    poses = json.loads(args.poses.read_text())
    lighting = json.loads(args.lighting.read_text())
    refresh=None
    compatibility=None
    calibration_hash=source_hash
    if args.refresh_from:
        refresh=json.loads(args.refresh_from.read_text())
        if refresh.get('baked_detail') is not False:
            raise ValueError('Texture-only refresh requires an original beauty with no baked decal')
        calibration_hash=digest(args.baseline_source)
        if refresh['source_sha256']!=calibration_hash or refresh['base_image_sha256']!=beauty_hash or refresh['response_kind']!='vinyl_detail':
            raise ValueError('Refresh response must match the original source and beauty')
        current_scene,current_detail=signature(),detail_response_signature()
        bpy.ops.wm.open_mainfile(filepath=str(args.baseline_source.resolve()))
        base_scene=signature()
        scene_matches=current_scene==base_scene
        if current_detail!=detail_response_signature() or (not scene_matches and not args.overlay_only):
            raise ValueError('Scene or detail geometry/shader changed; perform a normal matched scene rebake')
        canonical=lambda value:hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        compatibility={'scope':'vinyl-detail-texture-and-opacity-only',
                       'base_source_sha256':calibration_hash,'updated_source_sha256':source_hash,
                       'unchanged_scene_sha256':canonical(base_scene),
                       'unchanged_detail_response_sha256':canonical(current_detail),
                       'source_scene_matches':scene_matches,
                       'other_source_changes_excluded':not scene_matches,
                       'base_response_sha256':digest(args.refresh_from)}
        bpy.ops.wm.open_mainfile(filepath=str(source.resolve()))
    if any(m['source_sha256'] != calibration_hash for m in (baseline, poses, lighting)):
        raise ValueError('All packages must match the source')
    if baseline['image_sha256'] != beauty_hash or lighting['base_image_sha256'] != beauty_hash:
        raise ValueError('Beauty/lighting mismatch')
    scene = bpy.context.scene
    if scene.use_nodes or scene.camera.name != baseline['camera']:
        raise ValueError('Expected authored camera and no compositor')
    detail = bpy.data.objects['Vinyl detail - dust and minor scratches']
    baked_detail = detail.visible_camera and not detail.hide_render and not any(c.hide_render for c in detail.users_collection)
    # The supplied scenes intentionally disable this collection for the beauty.
    # Enable it only in this non-saving export for references/calibration.
    for collection in detail.users_collection:
        collection.hide_render = False
    material = detail.data.materials[0].copy()
    detail.data.materials[0] = material
    tex = next(n for n in material.node_tree.nodes if n.type == 'TEX_IMAGE')
    opacity = material.node_tree.nodes['Vinyl detail opacity']
    if opacity.operation != 'MULTIPLY' or not 0 <= opacity.inputs[1].default_value <= 1:
        raise ValueError('Expected authored multiplicative decal alpha')
    authored_opacity = float(opacity.inputs[1].default_value)
    image = tex.image
    if not image.packed_file or tuple(image.size) != (2048, 2048):
        raise ValueError('Expected packed 2048px surface texture')
    # Packed byte-image pixels are bottom-up. Decode and premultiply BEFORE
    # building the runtime mip chain to avoid halos around sparse bright marks.
    data = np.empty(2048*2048*4, dtype=np.float32)
    image.pixels.foreach_get(data)
    data = data.reshape(2048, 2048, 4)[::-1].copy()
    # Byte-backed packed images expose encoded pixel values through bpy.pixels;
    # Cycles' Image Texture node decodes sRGB before shading. Match that here.
    if image.colorspace_settings.name != 'sRGB' or image.is_float:
        raise ValueError('Expected byte-backed sRGB detail image')
    rgb=data[..., :3]
    data[..., :3]=np.where(rgb<=.04045,rgb/12.92,((rgb+.055)/1.055)**2.4)
    data[..., 3] *= authored_opacity
    data[..., :3] *= data[..., 3:4]
    if not np.isfinite(data).all() or data.min() < 0 or data.max() > 1:
        raise ValueError('Invalid linear decal texture')
    args.output.mkdir(parents=True, exist_ok=True)
    work = args.output/'work'
    work.mkdir(exist_ok=True)
    texture = args.output/'detail.rgba8'
    texture.write_bytes(np.rint(data*255).astype('u1').tobytes())
    # UV top-left, top-right, bottom-right, bottom-left; matches the artwork's
    # clockwise angle without depending on editable artwork size/corners.
    corners = []
    for i in (3, 2, 1, 0):
        p = world_to_camera_view(scene, scene.camera, detail.matrix_world@detail.data.vertices[i].co)
        corners.append([p.x, 1-p.y])
    if refresh:
        if not np.allclose(corners,refresh['corners'],rtol=0,atol=1e-8):
            raise ValueError('Projected detail geometry differs from original response')
        # Keep the proven scene calibration/hash; provenance for the new overlay
        # is separate. Never pretend the old room/arm images were newly rendered.
        for key in ('illumination','residual','view_lut'):
            entry=refresh[key]
            original=args.refresh_from.parent/entry['file']
            if original.parent.resolve()!=args.refresh_from.parent.resolve() or digest(original)!=entry['sha256']:
                raise ValueError('Invalid or corrupt original response buffer')
            (args.output/entry['file']).write_bytes(original.read_bytes())
        refresh.update(source=str(args.baseline_source.resolve()),
                       overlay_source=str(source),overlay_source_sha256=source_hash,
                       overlay_compatibility=compatibility,
                       detail={'file':texture.name,'sha256':digest(texture)},opacity=authored_opacity)
        if digest(source)!=source_hash or digest(args.beauty)!=beauty_hash:
            raise ValueError('Source or beauty changed during texture refresh')
        (args.output/'atlas.json').write_text(json.dumps(refresh,indent=2)+'\n')
        print('TALIS_VINYL_REFRESH '+json.dumps({'opacity':authored_opacity,'overlay_source_sha256':source_hash}),flush=True)
        return
    width, height = baseline['resolution']
    crop = lighting['crop']
    left, top, w, h = crop
    for name in poses['moving_objects']:
        bpy.data.objects[name].hide_render = True
    scene.render.engine = 'CYCLES'
    configure_device(scene, args.device)
    configure_quality(scene, args)
    scene.render.use_persistent_data = True
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.use_crop_to_border = True
    scene.render.border_min_x, scene.render.border_max_x = left/width, (left+w)/width
    scene.render.border_min_y, scene.render.border_max_y = 1-(top+h)/height, 1-top/height
    scene.use_nodes = True
    aov = bpy.context.view_layer.aovs.add()
    aov.name, aov.type = 'TalisVinylCoverage', 'VALUE'
    tree = scene.node_tree
    tree.nodes.clear()
    render = tree.nodes.new('CompositorNodeRLayers')
    composite = tree.nodes.new('CompositorNodeComposite')
    tree.links.new(render.outputs['Image'], composite.inputs['Image'])
    output = tree.nodes.new('CompositorNodeOutputFile')
    output.base_path = str(work.resolve())
    output.format.file_format = 'OPEN_EXR'
    output.format.color_mode, output.format.color_depth = 'RGBA', '32'
    tree.links.new(render.outputs['Image'], output.inputs[0])
    scene.render.image_settings.file_format = 'PNG'
    scene.render.image_settings.color_mode = 'RGB'

    def render_frame(name):
        output.file_slots[0].path = name+'-linear-'
        scene.render.filepath = str((work/(name+'.png')).resolve())
        print('TALIS_VINYL_RENDER '+name, flush=True)
        bpy.ops.render.render(write_still=True)
        return pixels(work/f'{name}-linear-{scene.frame_current:04}.exr')

    if args.references:
        render_frame('reference-zero')
        # Positive screen-space angle is clockwise; Blender's world-Z positive
        # rotation is counter-clockwise. Match the runtime's visible direction.
        detail.parent.rotation_euler.z -= 1.2
        bpy.context.view_layer.update()
        render_frame('reference-rotated')
        detail.parent.rotation_euler.z += 1.2
        bpy.context.view_layer.update()
    if args.references_only:
        return
    detail.hide_render = True
    clean = render_frame('clean')
    detail.hide_render = False
    # Camera-only opaque diffuse white sheet gives fixed illumination; secondary
    # rays continue to see the untouched record rather than a large white card.
    nodes, links = material.node_tree.nodes, material.node_tree.links
    nodes.clear()
    diffuse = nodes.new('ShaderNodeBsdfDiffuse')
    diffuse.inputs['Color'].default_value = (1, 1, 1, 1)
    transparent = nodes.new('ShaderNodeBsdfTransparent')
    path = nodes.new('ShaderNodeLightPath')
    mix = nodes.new('ShaderNodeMixShader')
    links.new(path.outputs['Is Camera Ray'], mix.inputs[0])
    links.new(transparent.outputs[0], mix.inputs[1])
    links.new(diffuse.outputs[0], mix.inputs[2])
    out = nodes.new('ShaderNodeOutputMaterial')
    links.new(mix.outputs[0], out.inputs['Surface'])
    aov_node = nodes.new('ShaderNodeOutputAOV')
    aov_node.name = aov.name
    aov_node.inputs['Value'].default_value = 1
    output.file_slots.new('coverage-')
    links = tree.links
    links.new(render.outputs[aov.name], output.inputs[1])
    light = render_frame('white')
    mask = pixels(work/f'coverage-{scene.frame_current:04}.exr')[..., 0]
    if float(mask.max()) < .999 or np.count_nonzero(mask > .999) < 1000:
        raise ValueError('Detail plane is not visible in calibration')
    illumination = np.maximum(light[..., :3] - clean[..., :3]*(1-mask[..., None]), 0)
    maps = {'illumination.rgba16f': np.dstack((illumination, np.clip(mask, 0, 1))),
            'residual.rgba16f': np.dstack((np.maximum(clean[..., :3], 0), np.ones_like(mask)))}
    files = {}
    for name, values in maps.items():
        values = values.astype('<f2')
        if values.shape != (h, w, 4) or not np.isfinite(values).all():
            raise ValueError('Invalid response map')
        (args.output/name).write_bytes(values.tobytes())
        files[name] = {'file': name, 'sha256': digest(args.output/name)}
    lut = args.output/'view.rgba8'
    lut.write_bytes((args.lighting.parent/lighting['view_lut']['file']).read_bytes())
    metadata = dict(lighting)
    metadata.update(response_kind='vinyl_detail', corners=corners,
                    illumination=files['illumination.rgba16f'], residual=files['residual.rgba16f'],
                    view_lut={'file': lut.name, 'sha256': digest(lut)},
                    detail={'file': texture.name, 'sha256': digest(texture)}, detail_size=2048,
                    baked_detail=baked_detail,
                    opacity=authored_opacity,
                    quality=quality_metadata(scene),
                    limitations=['Fixed lighting and approximate existing-arm-shadow attenuation',
                                 'Surface marks rotate; underlying grooves, rim and reflection remain baked'])
    if digest(source) != source_hash or digest(args.beauty) != beauty_hash:
        raise ValueError('Source or beauty changed during export')
    (args.output/'atlas.json').write_text(json.dumps(metadata, indent=2)+'\n')
    print('TALIS_VINYL_COMPLETE '+json.dumps({'crop': crop, 'corners': corners}), flush=True)


if __name__ == '__main__':
    main()
