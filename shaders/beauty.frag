#version 330 core
in vec2 vUv;
uniform sampler2D uBeauty;
layout(location=0) out vec4 fragColor;
layout(location=1) out float foregroundMask;
// ARTWORK_FUNCTIONS
// BUTTON_FUNCTIONS
// KNOB_FUNCTIONS
void main() {
    // Blender already applied AgX and display encoding when saving the PNG.
    // RGBA8 storage + disabled FRAMEBUFFER_SRGB: do not tone-map/encode again.
    fragColor = vec4(applySleeve(texture(uBeauty, vUv).rgb, vUv), 1.0);
    foregroundMask = 0.0;
    fragColor.rgb = applyButtons(fragColor.rgb, vUv);
    fragColor.rgb = applyKnob(fragColor.rgb, vUv);
}
