// Stationary calibrated paper response; only the artwork UV rotates.
uniform bool uRecordEnabled;
uniform float uRecordBrightness; // Night-only center-label trim; never sleeve/dust.
uniform sampler2D uRecordIllumination;
uniform sampler2D uRecordResidual;
uniform sampler3D uRecordView;
uniform vec4 uRecordCrop;
uniform vec3 uRecordLut;

vec3 recordColour(vec3 encoded, vec2 sceneUv, vec3 scene) {
    if (!uRecordEnabled) return encoded*uRecordBrightness;
    vec2 patchUv = (sceneUv-uRecordCrop.xy)/uRecordCrop.zw;
    if (any(lessThan(patchUv,vec2(0.0))) || any(greaterThan(patchUv,vec2(1.0)))) return scene;
    vec4 illumination = textureLod(uRecordIllumination,patchUv,0.0);
    // The calibration's coverage protects the stationary spindle. Its edge
    // coverage is already baked into illumination/residual, just like the sleeve.
    if (illumination.a <= 0.0001) return scene;
    vec3 linear = sleeveLinear(encoded)*illumination.rgb
                + textureLod(uRecordResidual,patchUv,0.0).rgb;
    vec3 logUv = clamp((log2(max(linear,vec3(exp2(uRecordLut.x))))
                       -uRecordLut.x)*uRecordLut.y,0.0,1.0);
    return textureLod(uRecordView,(logUv*(uRecordLut.z-1.0)+0.5)/uRecordLut.z,0.0).rgb*uRecordBrightness;
}
