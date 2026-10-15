"""Conservative evaluated scene signature for verifying overlay-only refreshes.

Includes visible and secondary-ray geometry, UVs/normals, materials/textures,
light/camera/world, transforms, collection visibility and authored view settings.
Excludes only the named vinyl detail collection and render-quality/UI settings.
No source is saved. Unknown differences require a normal scene rebake.
"""
import hashlib
import json
import sys
from pathlib import Path

import bpy
import numpy as np

DETAIL_COLLECTION = 'VINYL - rotating surface detail'


def array_hash(collection, prop, count, dtype):
    values = np.empty(count, dtype=dtype)
    collection.foreach_get(prop, values)
    return hashlib.sha256(values.tobytes()).hexdigest()


def properties(value, ignore=()):
    result = {}
    for p in value.bl_rna.properties:
        name = p.identifier
        if name in ignore or name in {'rna_type', 'name', 'name_full', 'type', 'is_updated', 'is_updated_data',
                                    'is_updated_transform', 'is_evaluated', 'original', 'users', 'use_fake_user',
                                    'tag', 'is_embedded_data', 'is_library_indirect', 'library', 'override_library',
                                    'asset_data', 'animation_data', 'preview', 'id_data', 'session_uid'}:
            continue
        if p.type not in {'BOOLEAN','INT','FLOAT','STRING','ENUM'}:
            continue
        v = getattr(value, name)
        if getattr(p, 'is_array', False): v = list(v)
        if isinstance(v, set): v = sorted(v)
        result[name] = v
    return result


def node_tree(tree):
    if tree is None: return None
    nodes = {}
    ignored = {'label','location','width','height','width_hidden','dimensions','select','show_options',
               'show_preview','hide','show_texture','use_custom_color','color','show_expanded', 'warning_propagation'}
    for n in tree.nodes:
        data = {'type':n.bl_idname, 'properties':properties(n,ignored),
                'inputs':{str(i)+':'+s.name:list(s.default_value) if hasattr(s.default_value,'__len__') and not isinstance(s.default_value,str)
                          else s.default_value for i,s in enumerate(n.inputs) if hasattr(s,'default_value')}}
        data['id_references']={p.identifier:getattr(n,p.identifier).name_full for p in n.bl_rna.properties
                               if p.type=='POINTER' and isinstance(getattr(n,p.identifier,None),bpy.types.ID)
                               and p.identifier not in {'image','node_tree','id_data'}}
        for name in ('texture_mapping','color_mapping'):
            if hasattr(n,name): data[name]=properties(getattr(n,name))
        if getattr(n,'image',None):
            im=n.image
            packed=[hashlib.sha256(p.packed_file.data).hexdigest() for p in im.packed_files]
            if not packed and im.source=='FILE':
                path=Path(bpy.path.abspath(im.filepath))
                packed=[hashlib.sha256(path.read_bytes()).hexdigest()]
            data['image']={'content':packed,'size':list(im.size),'colour_space':im.colorspace_settings.name,'alpha_mode':im.alpha_mode}
        if getattr(n,'node_tree',None): data['group']=node_tree(n.node_tree)
        if hasattr(n,'color_ramp'):
            data['ramp']={'properties':properties(n.color_ramp), 'elements':[(e.position,list(e.color)) for e in n.color_ramp.elements]}
        nodes[n.name]=data
    return {'nodes':nodes,'links':sorted((l.from_node.name,l.from_socket.identifier,l.to_node.name,l.to_socket.identifier) for l in tree.links)}


def attributes(mesh):
    result={}
    for attr in mesh.attributes:
        data={'type':attr.data_type,'domain':attr.domain,'count':len(attr.data)}
        if attr.data:
            item=attr.data[0]
            name=next((key for key in ('value','vector','color') if hasattr(item,key)),None)
            if name is None: raise ValueError('Unsupported evaluated mesh attribute: '+attr.name)
            p=item.bl_rna.properties[name]
            if p.type=='STRING':
                data['values']=hashlib.sha256(json.dumps([getattr(i,name) for i in attr.data]).encode()).hexdigest()
            else:
                dtype={'FLOAT':np.float32,'INT':np.int32,'BOOLEAN':np.bool_}.get(p.type)
                if dtype is None: raise ValueError('Unsupported mesh attribute type: '+attr.name)
                data['values']=array_hash(attr.data,name,len(attr.data)*(p.array_length if p.is_array else 1),dtype)
        result[attr.name]=data
    return result


def signature():
    scene=bpy.context.scene
    deps=bpy.context.evaluated_depsgraph_get()
    excluded={o.name for c in bpy.data.collections if c.name==DETAIL_COLLECTION for o in c.all_objects}
    objects={}; materials={}
    for obj in sorted(scene.objects,key=lambda o:o.name):
        if obj.name in excluded: continue
        data={'type':obj.type,'matrix':list(map(list,obj.matrix_world)),
              'visibility':{k:getattr(obj,k) for k in ('hide_render','visible_camera','visible_diffuse','visible_glossy','visible_transmission','visible_volume_scatter','visible_shadow')},
              'collections':sorted(c.name for c in obj.users_collection)}
        evaluated=obj.evaluated_get(deps)
        if obj.type in {'MESH','FONT','CURVE','SURFACE','META'}:
            mesh=evaluated.to_mesh()
            if mesh:
                data['mesh']={'counts':[len(mesh.vertices),len(mesh.loops),len(mesh.polygons)],
                    'vertices':array_hash(mesh.vertices,'co',len(mesh.vertices)*3,np.float32),
                    'normals':array_hash(mesh.loops,'normal',len(mesh.loops)*3,np.float32),
                    'loops':array_hash(mesh.loops,'vertex_index',len(mesh.loops),np.int32),
                    'faces':array_hash(mesh.polygons,'loop_total',len(mesh.polygons),np.int32),
                    'material_indices':array_hash(mesh.polygons,'material_index',len(mesh.polygons),np.int32),
                    'smooth':array_hash(mesh.polygons,'use_smooth',len(mesh.polygons),np.bool_),
                    'uv':{uv.name:array_hash(uv.data,'uv',len(uv.data)*2,np.float32) for uv in mesh.uv_layers}}
                data['materials']=[m.name if m else None for m in mesh.materials]
                data['attributes']=attributes(mesh)
                for m in mesh.materials:
                    if m and m.name not in materials:
                        materials[m.name]={'properties':properties(m,{'diffuse_color'}),'nodes':node_tree(m.node_tree)}
                evaluated.to_mesh_clear()
        elif obj.data:
            data['data']=properties(obj.data)
            if hasattr(obj.data,'node_tree'): data['nodes']=node_tree(obj.data.node_tree)
        objects[obj.name]=data
    def layers(l):
        if l.name==DETAIL_COLLECTION: return None
        return {'name':l.name,'exclude':l.exclude,'holdout':l.holdout,'indirect_only':l.indirect_only,
                'hide_render':l.collection.hide_render,'children':[v for c in l.children if (v:=layers(c)) is not None]}
    return {'objects':objects,'materials':materials,'world':{'properties':properties(scene.world),'nodes':node_tree(scene.world.node_tree)},
            'camera':scene.camera.name,'view':properties(scene.view_settings),'display':properties(scene.display_settings),
            'frame':scene.frame_current,'compositor':node_tree(scene.node_tree) if scene.use_nodes else None,
            'resolution':[scene.render.resolution_x,scene.render.resolution_y,scene.render.resolution_percentage,
                          scene.render.pixel_aspect_x,scene.render.pixel_aspect_y],
            'layers':[layers(l.layer_collection) for l in scene.view_layers]}


def detail_response_signature():
    """Changes to pixels and alpha gain are allowed; response geometry/shader is not."""
    collection=bpy.data.collections[DETAIL_COLLECTION]
    result={'hide_render':collection.hide_render,'objects':{}}
    deps=bpy.context.evaluated_depsgraph_get()
    for obj in sorted(collection.all_objects,key=lambda o:o.name):
        data={'type':obj.type,'matrix':list(map(list,obj.matrix_world)),
              'visibility':{k:getattr(obj,k) for k in ('hide_render','visible_camera','visible_diffuse','visible_glossy','visible_transmission','visible_volume_scatter','visible_shadow')}}
        if obj.type=='MESH':
            evaluated=obj.evaluated_get(deps);mesh=evaluated.to_mesh()
            data['geometry']={'vertices':array_hash(mesh.vertices,'co',len(mesh.vertices)*3,np.float32),
                'normals':array_hash(mesh.loops,'normal',len(mesh.loops)*3,np.float32),
                'loops':array_hash(mesh.loops,'vertex_index',len(mesh.loops),np.int32),
                'uv':{uv.name:array_hash(uv.data,'uv',len(uv.data)*2,np.float32) for uv in mesh.uv_layers}}
            data['attributes']=attributes(mesh)
            data['materials']=[]
            for material in mesh.materials:
                tree=node_tree(material.node_tree)
                for node in tree['nodes'].values():
                    if node['type']=='ShaderNodeTexImage': node.pop('image',None)
                tree['nodes']['Vinyl detail opacity']['inputs'].pop('1:Value')
                data['materials'].append(tree)
            evaluated.to_mesh_clear()
        result['objects'][obj.name]=data
    return result


if __name__=='__main__':
    output=Path(sys.argv[sys.argv.index('--')+1])
    output.write_text(json.dumps(signature(),sort_keys=True,indent=2)+'\n')
    print('TALIS_SIGNATURE '+str(output),flush=True)
