// Circular artwork in the record plane, with optional stationary paper lighting.
uniform sampler2D uArtwork;
uniform bool uArtworkEnabled;
uniform mat3 uArtworkTransform;
uniform vec2 uArtworkRotation; // cosine/sine of clockwise record-plane angle
// SLEEVE_FUNCTIONS
// RECORD_LIGHTING_FUNCTIONS
vec3 applyRecordArtwork(vec3 scene, vec2 sceneUv, float foreground) {
    if (!uArtworkEnabled) return scene;
    vec3 projected = uArtworkTransform * vec3(sceneUv, 1.0);
    if (projected.z <= 0.000001) return scene;
    vec2 uv = projected.xy / projected.z;
    float radius = length(uv - vec2(0.5));
    // Pixel-sized antialiasing of the cut edge, not a blur/glow effect.
    float edge = max(fwidth(radius), 0.00001);
    float coverage = 1.0 - smoothstep(0.5-edge*0.5, 0.5+edge*0.5, radius);
    vec2 centered = uv - vec2(0.5);
    uv = vec2(uArtworkRotation.x*centered.x + uArtworkRotation.y*centered.y,
             -uArtworkRotation.y*centered.x + uArtworkRotation.x*centered.y) + vec2(0.5);
    // Compute the rotated/projected footprint before the circular-edge branch.
    // Explicit gradients keep mip selection defined even at divergent edge pixels.
    vec2 uvDx = dFdx(uv);
    vec2 uvDy = dFdy(uv);
    if (coverage <= 0.0) return scene;
    vec4 art = textureGrad(uArtwork, uv, uvDx, uvDy);
    vec3 colour = recordColour(art.rgb,sceneUv,scene);
    return mix(scene, colour, coverage * art.a * (1.0-clamp(foreground,0.0,1.0)));
}
