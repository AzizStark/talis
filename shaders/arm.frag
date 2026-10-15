#version 330 core
in vec2 vUv;
uniform sampler2D uBeauty;
uniform sampler2DArray uPoses;
uniform vec4 uCrop; // normalized top-left x,y,width,height
uniform vec4 uUpdateCrop;
uniform vec2 uCropPixels;
uniform float uPoseLayer; // one nearest integer layer; no temporal crossfade
layout(location=0) out vec4 fragColor;
layout(location=1) out float foregroundMask;
// ARTWORK_FUNCTIONS
// BUTTON_FUNCTIONS
// KNOB_FUNCTIONS
void main() {
    vec3 color = texture(uBeauty, vUv).rgb;
    vec2 uv = (vUv-uCrop.xy)/uCrop.zw;
    float foreground = 0.0;
    vec2 updateUv=(vUv-uUpdateCrop.xy)/uUpdateCrop.zw;
    if (all(greaterThanEqual(uv,vec2(0.0))) && all(lessThanEqual(uv,vec2(1.0)))
        && all(greaterThanEqual(updateUv,vec2(0.0))) && all(lessThanEqual(updateUv,vec2(1.0)))) {
        vec4 samplePose = texture(uPoses,vec3(uv,uPoseLayer));
        vec3 pose = samplePose.rgb;
        vec2 distance = min(uv,1.0-uv)*uCropPixels;
        // Conservative crop margin keeps all moving details outside this seam feather.
        float coverage = smoothstep(0.0,16.0,min(distance.x,distance.y));
        vec2 updateDistance=min(updateUv,1.0-updateUv)*uUpdateCrop.zw/uCrop.zw*uCropPixels;
        coverage=min(coverage,smoothstep(0.0,16.0,min(updateDistance.x,updateDistance.y)));
        color = mix(color,pose,coverage);
        foreground = samplePose.a * coverage;
    }
    // Each complete Blender patch is already AgX/display-encoded. Keep the selected
    // pose intact; only the static crop boundary is feathered into the beauty.
    fragColor = vec4(applySleeve(color,vUv),1.0);
    foregroundMask = foreground;
    fragColor.rgb = applyButtons(fragColor.rgb, vUv);
    fragColor.rgb = applyKnob(fragColor.rgb, vUv);
}
