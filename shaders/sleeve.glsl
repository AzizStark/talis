// Linear printed-paper transfer, evaluated only inside the baked sleeve patch.
uniform bool uSleeveEnabled;
uniform sampler2D uSleeveIllumination;
uniform sampler2D uSleeveResidual;
uniform sampler3D uSleeveView;
uniform vec4 uSleeveCrop;
uniform mat3 uSleeveTransform;
uniform vec3 uSleeveLut; // log minimum, inverse log range, edge length
uniform vec3 uSleeveLightReference;
uniform float uSleeveShadowLift;

vec3 sleeveLighting(vec4 illumination) {
    // An artistic, shadow-selective adjustment to the diffuse paper response,
    // not an exposure lift of dark artwork. Do not touch the residual (coat,
    // highlights and background), the rotating label, or other scene pixels.
    if (uSleeveShadowLift <= 0.0 || all(lessThanEqual(uSleeveLightReference,vec3(0.0))))
        return illumination.rgb;
    // Approach this scene's bright-paper RGB, not a scaled cool shadow tint.
    // Each channel is continuous and monotonic: no narrow luminance gate that
    // can reverse the gradient and leave a contour at high lift. Brighter light
    // is retained, and premultiplied coverage still protects the rounded edge.
    vec3 target = uSleeveLightReference*illumination.a;
    return mix(illumination.rgb,max(illumination.rgb,target),
               clamp(uSleeveShadowLift,0.0,1.0));
}

vec3 sleeveLinear(vec3 encoded) {
    return mix(encoded/12.92, pow((encoded+0.055)/1.055, vec3(2.4)),
               step(vec3(0.04045),encoded));
}
vec3 applySleeve(vec3 scene, vec2 sceneUv) {
    if (!uSleeveEnabled) return scene;
    // Take scene derivatives before any varying coverage/ROI branch. Derivatives
    // inside divergent flow are undefined, especially at the paper's rounded edge.
    vec2 screenDx = dFdx(sceneUv), screenDy = dFdy(sceneUv);
    vec2 patchUv = (sceneUv-uSleeveCrop.xy)/uSleeveCrop.zw;
    if (any(lessThan(patchUv,vec2(0.0))) || any(greaterThan(patchUv,vec2(1.0)))) return scene;
    vec4 illumination = textureLod(uSleeveIllumination,patchUv,0.0);
    if (illumination.a <= 0.0001) return scene;
    vec3 projected = uSleeveTransform * vec3(sceneUv,1.0);
    if (projected.z <= 0.000001) return scene;
    vec2 uv = projected.xy/projected.z;
    // Clamp edge UVs: the rounded geometry and occlusion are in the baked mask.
    // This image is stationary; record size and rotation never affect its fit.
    vec3 projectedDx = uSleeveTransform * vec3(screenDx,0.0);
    vec3 projectedDy = uSleeveTransform * vec3(screenDy,0.0);
    vec2 artDx = (projectedDx.xy-uv*projectedDx.z)/projected.z;
    vec2 artDy = (projectedDy.xy-uv*projectedDy.z)/projected.z;
    vec4 art = textureGrad(uArtwork,clamp(uv,vec2(0.0),vec2(1.0)),artDx,artDy);
    vec3 linear = sleeveLinear(art.rgb)*sleeveLighting(illumination)
                + textureLod(uSleeveResidual,patchUv,0.0).rgb;
    vec3 logUv = clamp((log2(max(linear,vec3(exp2(uSleeveLut.x))))
                       - uSleeveLut.x)*uSleeveLut.y,0.0,1.0);
    vec3 lookup = (logUv*(uSleeveLut.z-1.0)+0.5)/uSleeveLut.z;
    vec3 lit = textureLod(uSleeveView,lookup,0.0).rgb;
    // Coverage is already present in the calibrated response and background
    // residual. Multiplying by illumination.a again would double-AA the edge.
    return mix(scene,lit,art.a);
}
