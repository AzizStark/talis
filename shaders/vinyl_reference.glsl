// Scene-fixed calibration, shared by the live fallback and cached reference pass.
uniform bool uVinylBakedDetail;
uniform sampler2D uVinylDetail;
uniform sampler2D uVinylIllumination;
uniform sampler2D uVinylResidual;
uniform sampler3D uVinylView;
uniform vec4 uVinylCrop;
uniform vec3 uVinylLut;
uniform mat3 uVinylTransform;

vec3 vinylView(vec3 linear) {
    vec3 uv = clamp((log2(max(linear,vec3(exp2(uVinylLut.x))))-uVinylLut.x)*uVinylLut.y,0.0,1.0);
    return textureLod(uVinylView,(uv*(uVinylLut.z-1.0)+0.5)/uVinylLut.z,0.0).rgb;
}
vec3 vinylReference(vec3 base, vec3 light, vec4 original) {
    return vinylView(uVinylBakedDetail ? base*(1.0-original.a)+light*original.rgb : base);
}
