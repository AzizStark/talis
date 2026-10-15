#version 330 core
in vec2 vUv;
uniform sampler2D uBeauty;
uniform sampler2DArray uPoses;
uniform sampler2DArray uFlow;
uniform sampler2D uTubeBackground;
uniform bool uEdgeLayers;
uniform vec4 uCrop;
uniform vec4 uUpdateCrop;
uniform vec2 uCropPixels;
uniform vec3 uPose; // lower image, upper image, fraction
uniform float uFlowPair;
layout(location=0) out vec4 fragColor;
layout(location=1) out float foregroundMask;
// ARTWORK_FUNCTIONS
// BUTTON_FUNCTIONS
// KNOB_FUNCTIONS

#define armSample texture
vec4 motion(vec2 p) {
    return (armSample(uFlow,vec3(p,uFlowPair))*65535.0-32768.0)/256.0;
}
float inside(vec2 p) {
    return all(greaterThanEqual(p,vec2(0.0))) && all(lessThanEqual(p,vec2(1.0))) ? 1.0 : 0.0;
}
const float stationaryMin=0.5/255.0;
const float stationaryMax=2.0/255.0;
vec3 interpolate(vec2 p, out float foreground) {
    float t=uPose.z;
    // Authored poses remain exact; do not warp an endpoint or the last image.
    if(t<=0.000001) {
        vec4 authored=armSample(uPoses,vec3(p,uPose.x));
        foreground=authored.a;
        return authored.rgb;
    }
    vec2 a=p,b=p;
    // Inverse gather: solve p = source + fraction * source-space displacement.
    for(int i=0;i<3;i++) {
        a=p-t*motion(a).xy/uCropPixels;
        b=p-(1.0-t)*motion(b).zw/uCropPixels;
    }
    vec2 forward=motion(a).xy,backward=motion(b).zw;
    float errorA=length(forward+motion(a+forward/uCropPixels).zw);
    float errorB=length(backward+motion(b+backward/uCropPixels).xy);
    // Keep low-confidence motion-aligned samples available. Dropping them to
    // zero and switching to an unwarped pose makes thin tubes split and snap.
    float wa=(1.0-t)*mix(0.12,1.0,1.0-smoothstep(0.75,2.5,errorA))*inside(a);
    float wb=t*mix(0.12,1.0,1.0-smoothstep(0.75,2.5,errorB))*inside(b);
    vec4 poseA=armSample(uPoses,vec3(a,uPose.x));
    vec4 poseB=armSample(uPoses,vec3(b,uPose.y));
    vec3 ca=poseA.rgb,cb=poseB.rgb;
    if(uEdgeLayers) {
        vec4 bg=armSample(uTubeBackground,p);
        vec4 bgA=armSample(uTubeBackground,a);
        vec4 bgB=armSample(uTubeBackground,b);
        // Reposition only the background contribution in antialiased edge pixels.
        // Coverage one preserves the arm; coverage zero keeps static disk geometry
        // at p while retaining the source image's changing illumination residual.
        ca=clamp(ca+(bg.rgb-bgA.rgb)*(1.0-poseA.a)*min(bg.a,bgA.a),0.0,1.0);
        cb=clamp(cb+(bg.rgb-bgB.rgb)*(1.0-poseB.a)*min(bg.a,bgB.a),0.0,1.0);
    }
    // Continuously favor the stronger sample at conflicting edges. A hard
    // mismatch threshold used to switch neighboring tube pixels independently.
    vec3 delta=abs(ca-cb);
    float mismatch=max(delta.r,max(delta.g,delta.b));
    float preference=mix(1.0,3.0,smoothstep(0.12,0.30,mismatch));
    wa=pow(wa,preference);
    wb=pow(wb,preference);
    foreground=wa+wb<0.000001 ? mix(poseA.a,poseB.a,t) : (poseA.a*wa+poseB.a*wb)/(wa+wb);
    vec3 moving=wa+wb<0.000001 ? mix(ca,cb,t) : (ca*wa+cb*wb)/(wa+wb);
    // Motion fields extend beyond the tube so inverse gathers remain coherent.
    // Do not let that margin drag a stationary record/pedestal edge with it.
    // Gather these after warping to keep their live shader state short.
    vec3 fixedA=armSample(uPoses,vec3(p,uPose.x)).rgb;
    vec3 fixedB=armSample(uPoses,vec3(p,uPose.y)).rgb;
    vec3 fixedDelta=abs(fixedA-fixedB);
    float change=max(fixedDelta.r,max(fixedDelta.g,fixedDelta.b));
    float stationary=1.0-smoothstep(stationaryMin,stationaryMax,change);
    return mix(moving,mix(fixedA,fixedB,t),stationary);
}
void main() {
    vec3 color=texture(uBeauty,vUv).rgb;
    vec2 uv=(vUv-uCrop.xy)/uCrop.zw;
    float foreground=0.0;
    vec2 updateUv=(vUv-uUpdateCrop.xy)/uUpdateCrop.zw;
    if(inside(uv)>0.5 && inside(updateUv)>0.5) {
        vec3 pose=interpolate(uv,foreground);
        vec2 distance=min(uv,1.0-uv)*uCropPixels;
        float coverage=smoothstep(0.0,16.0,min(distance.x,distance.y));
        vec2 updateDistance=min(updateUv,1.0-updateUv)*uUpdateCrop.zw/uCrop.zw*uCropPixels;
        coverage=min(coverage,smoothstep(0.0,16.0,min(updateDistance.x,updateDistance.y)));
        color=mix(color,pose,coverage);
        foreground*=coverage;
    }
    // Source RGB is already display-encoded. This is image synthesis, not live 3D lighting.
    fragColor=vec4(applySleeve(color,vUv),1.0);
    foregroundMask=foreground;
    fragColor.rgb = applyButtons(fragColor.rgb, vUv);
    fragColor.rgb = applyKnob(fragColor.rgb, vUv);
}
