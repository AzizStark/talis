#version 330 core
uniform sampler2D uSceneCache;
uniform sampler2D uForegroundCache;
uniform vec2 uCacheSize;
uniform vec2 uCacheOrigin;
uniform vec4 uSceneViewport;
out vec4 fragColor;
// ARTWORK_FUNCTIONS
// VINYL_FUNCTIONS
void main() {
    // Retained framebuffers are bottom-up. Sample the exact current pixel, not
    // an interpolated/rescaled arm edge; scene UV remains top-down for the label.
    vec2 pixelUv=(gl_FragCoord.xy+uCacheOrigin)/uCacheSize;
    vec3 scene=texture(uSceneCache,pixelUv).rgb;
    float foreground=texture(uForegroundCache,pixelUv).r;
    // Match global UV precision across full-size and cropped GTK targets.
    vec2 sceneUv=(gl_FragCoord.xy+uCacheOrigin-uSceneViewport.xy)/uSceneViewport.zw;
    sceneUv.y=1.0-sceneUv.y;
    scene=applyVinylDetail(scene,sceneUv,foreground);
    fragColor=vec4(applyRecordArtwork(scene,sceneUv,foreground),1.0);
}
