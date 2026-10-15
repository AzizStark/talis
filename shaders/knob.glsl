uniform sampler2D uKnobClean;
uniform sampler2D uKnobInk;
uniform bool uKnobEnabled;
uniform vec4 uKnobCrop;
uniform vec2 uKnobPixels;
uniform mat3 uKnobTransform;
uniform float uKnobAngle;
uniform vec3 uKnobMarker; // half-width, inner radius, outer radius
vec3 applyKnob(vec3 original, vec2 sceneUv) {
    if (!uKnobEnabled) return original;
    vec2 patch=(sceneUv-uKnobCrop.xy)/uKnobCrop.zw;
    if (any(lessThan(patch,vec2(0))) || any(greaterThan(patch,vec2(1)))) return original;
    vec3 projected=uKnobTransform*vec3(sceneUv,1);
    vec2 q=projected.xy/projected.z*2.0-1.0;
    float c=cos(uKnobAngle), s=sin(uKnobAngle);
    // Inverse gather in the knob's physical top plane, not screen space.
    q=mat2(c,-s,s,c)*q;
    vec2 halfSize=vec2(uKnobMarker.x,(uKnobMarker.z-uKnobMarker.y)*0.5);
    vec2 d=abs(q-vec2(0,-(uKnobMarker.y+uKnobMarker.z)*0.5))-halfSize;
    float distance=max(d.x,d.y);
    float aa=max(fwidth(distance),0.00001);
    float mask=1.0-smoothstep(-aa*0.5,aa*0.5,distance);
    // Ink lighting is sampled at its destination and NEVER rotates with ink.
    vec3 color=mix(texture(uKnobClean,patch).rgb,texture(uKnobInk,patch).rgb,mask);
    vec2 edge=min(patch,1.0-patch)*uKnobPixels;
    return mix(original,color,smoothstep(0.0,3.0,min(edge.x,edge.y)));
}
