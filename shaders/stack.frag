#version 330 core
in vec2 vUv;
uniform sampler2D uBeauty;
uniform sampler2D uArtwork;

// PAPER_RESPONSE

layout(location=0) out vec4 fragColor;
layout(location=1) out float foregroundMask;
void main() {
    fragColor = vec4(applySleeve(texture(uBeauty, vUv).rgb, vUv), 1.0);
    foregroundMask = 0.0;
}
