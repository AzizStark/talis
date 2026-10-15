uniform sampler2DArray uButtons;
uniform bool uButtonsEnabled;
uniform vec4 uButtonsCrop;
uniform vec2 uButtonsPixels;
uniform ivec3 uButtonsLayers;

vec3 applyButtons(vec3 original, vec2 sceneUv) {
    if (!uButtonsEnabled) return original;
    vec2 uv = (sceneUv-uButtonsCrop.xy)/uButtonsCrop.zw;
    if (any(lessThan(uv,vec2(0.0))) || any(greaterThan(uv,vec2(1.0)))) return original;
    vec3 neutral = texture(uButtons,vec3(uv,0.0)).rgb;
    vec3 color = neutral;
    // Independent real-depth patches; never crossfade endpoints or accumulate
    // into an earlier framebuffer. Zero-depth always shares the neutral layer.
    for (int i=0; i<3; ++i) {
        if (uButtonsLayers[i]>0)
            color += texture(uButtons,vec3(uv,float(uButtonsLayers[i]))).rgb-neutral;
    }
    vec2 edge = min(uv,1.0-uv)*uButtonsPixels;
    float coverage = smoothstep(0.0,3.0,min(edge.x,edge.y));
    return mix(original,clamp(color,0.0,1.0),coverage);
}
