#version 330 core
out vec2 vUv;
void main() {
    vec2 position = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    // PNG rows start at the top; OpenGL's framebuffer origin is bottom-left.
    vUv = vec2(position.x, 1.0 - position.y);
    gl_Position = vec4(position * 2.0 - 1.0, 0.0, 1.0);
}
