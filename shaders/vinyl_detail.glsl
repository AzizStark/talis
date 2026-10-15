// Sparse authored marks rotate, not the scene's grooves/reflections/shadows.
uniform bool uVinylEnabled;
// VINYL_REFERENCE_FUNCTIONS
// Opacity and linear-colour multipliers; original baked marks stay unadjusted
// for subtraction. Defaults (1,1) reproduce the authored surface exactly.
uniform vec2 uVinylAppearance;

vec3 applyVinylDetail(vec3 scene, vec2 sceneUv, float foreground) {
    if (!uVinylEnabled) return scene;
    vec3 p=uVinylTransform*vec3(sceneUv,1.0);
    if (p.z<=0.000001) return scene;
    vec2 uv=p.xy/p.z;
    float radius=length(uv-vec2(0.5));
    // Keep the geometry's rim and its AA pixels entirely in the cached beauty.
    if (radius>=0.493) return scene;
    vec2 patch=(sceneUv-uVinylCrop.xy)/uVinylCrop.zw;
    vec4 light=textureLod(uVinylIllumination,patch,0.0);
    // Calibration coverage excludes the fixed spindle and original paper label.
    if (light.a<0.999) return scene;
    vec2 centered=uv-vec2(0.5);
    vec2 rotated=vec2(uArtworkRotation.x*centered.x+uArtworkRotation.y*centered.y,
                     -uArtworkRotation.y*centered.x+uArtworkRotation.x*centered.y)+vec2(0.5);
    vec4 moving=texture(uVinylDetail,rotated);
    moving *= uVinylAppearance.x;
    moving.rgb *= uVinylAppearance.y;
    // Current unbaked packages need no reference at all on transparent marks.
    // Baked detail still needs original coverage to remove rotated-away marks.
    if (moving.a==0.0 && !uVinylBakedDetail) return scene;
    vec4 original=texture(uVinylDetail,uv);
    if (moving.a==0.0 && original.a==0.0) return scene;
    vec3 base=textureLod(uVinylResidual,patch,0.0).rgb;
    vec3 before=vinylReference(base,light.rgb,original);
    vec3 after=vinylView(base*(1.0-moving.a)+light.rgb*moving.rgb);
    // Existing arm-shadow attenuation, not a new live shadow. Reflections stay
    // in scene. When marks are already baked, zero angle is exactly identity.
    float luminance=dot(before,vec3(0.2126,0.7152,0.0722));
    float shadow=clamp(dot(scene,vec3(0.2126,0.7152,0.0722))/max(luminance,0.001),0.0,1.0);
    vec3 correction=(after-before)*shadow*shadow;
    float edge=1.0-smoothstep(0.487,0.493,radius);
    return clamp(scene+correction*edge*(1.0-clamp(foreground,0.0,1.0)),0.0,1.0);
}
