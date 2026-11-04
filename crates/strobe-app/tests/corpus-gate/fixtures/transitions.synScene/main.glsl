// The transition uniforms at their first values, split across the frame.
vec4 renderMain(void) {
    float left = step(_uv.x, 0.5);
    return vec4(mix(warm, side, left), cool, 0.25, 1.0);
}
